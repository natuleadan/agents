//! Service layer for singleton installation identity resolution, outbound injection, and indefinite ledgering.

use std::sync::Arc;

use agents_protocol::{
    HEADER_AGENT_IDENTITY_CODE, HEADER_AGENT_INSTALLATION_ID, HEADER_AGENT_ORGANIZATION,
    is_valid_alphanumeric_id, is_valid_organization_name,
};
use axum::http::{HeaderMap, HeaderName, HeaderValue};
use chrono::Utc;
use ironic::Injectable;
use serde_json::{Value, json};
use tracing::info;

use crate::infra::error::ServerError;

use super::{
    entities::{
        AgentInteractionLedgerEntry, InstallationIdentityRecord, LedgerListResponse,
        LedgerQueryDto, UpdateInstallationIdentityDto,
    },
    repositories::IdentityStore,
};

/// Parameters for recording an outbound external agent/webhook interaction.
pub struct OutboundInteractionRecord<'a> {
    pub protocol: &'a str,
    pub target_host: Option<&'a str>,
    pub endpoint_path: &'a str,
    pub method: &'a str,
    pub status_code: Option<i32>,
    pub task_id: Option<&'a str>,
    pub identity: Option<&'a InstallationIdentityRecord>,
}

/// Resolves installation identity (`.env` first, DB singleton fallback) and logs all external agent/webhook traffic.
#[derive(Clone, Injectable)]
pub struct IdentityService {
    store: Arc<IdentityStore>,
}

impl IdentityService {
    #[must_use]
    pub fn new(store: Arc<IdentityStore>) -> Self {
        Self { store }
    }

    /// Resolves the active installation identity: `.env` takes precedence over singleton DB config.
    pub async fn resolve_installation_identity(
        &self,
    ) -> Result<Option<InstallationIdentityRecord>, ServerError> {
        let env_org = std::env::var("AGENTS_ORG_NAME")
            .ok()
            .map(|s| s.trim().to_owned())
            .filter(|s| !s.is_empty());
        let env_id = std::env::var("AGENTS_INSTALLATION_ID")
            .ok()
            .map(|s| s.trim().to_owned())
            .filter(|s| !s.is_empty());

        if let (Some(organization_name), Some(installation_id)) = (env_org, env_id) {
            let identity_code = std::env::var("AGENTS_IDENTITY_CODE")
                .ok()
                .map(|s| s.trim().to_owned())
                .filter(|s| !s.is_empty());
            let registrar_url = std::env::var("AGENTS_IDENTITY_REGISTRAR_URL")
                .ok()
                .map(|s| s.trim().to_owned())
                .filter(|s| !s.is_empty());
            let strict_inbound_webhooks = std::env::var("AGENTS_STRICT_INBOUND_WEBHOOKS")
                .map(|v| matches!(v.to_ascii_lowercase().as_str(), "1" | "true" | "yes"))
                .unwrap_or(false);
            let strict_outbound_egress = std::env::var("AGENTS_STRICT_OUTBOUND_EGRESS")
                .map(|v| matches!(v.to_ascii_lowercase().as_str(), "1" | "true" | "yes"))
                .unwrap_or(true);

            return Ok(Some(InstallationIdentityRecord {
                organization_name,
                installation_id,
                has_identity_code: identity_code.is_some(),
                identity_code,
                registrar_url,
                strict_inbound_webhooks,
                strict_outbound_egress,
                source: "env".to_owned(),
                updated_at: Utc::now().to_rfc3339(),
            }));
        }

        self.store.0.get_identity().await
    }

    /// Updates the singleton installation identity stored in PostgreSQL.
    pub async fn update_installation_identity(
        &self,
        dto: &UpdateInstallationIdentityDto,
    ) -> Result<InstallationIdentityRecord, ServerError> {
        if !is_valid_organization_name(&dto.organization_name) {
            return Err(ServerError::Validation(
                "organizationName must be 2..=128 printable ASCII chars".into(),
            ));
        }
        if !is_valid_alphanumeric_id(&dto.installation_id) {
            return Err(ServerError::Validation(
                "installationId must be 4..=64 ASCII alphanumeric, '-' or '_' chars".into(),
            ));
        }

        let code = dto
            .identity_code
            .as_ref()
            .map(|s| s.trim().to_owned())
            .filter(|s| !s.is_empty());
        let record = InstallationIdentityRecord {
            organization_name: dto.organization_name.trim().to_owned(),
            installation_id: dto.installation_id.trim().to_owned(),
            has_identity_code: code.is_some(),
            identity_code: code,
            registrar_url: dto
                .registrar_url
                .as_ref()
                .map(|s| s.trim().to_owned())
                .filter(|s| !s.is_empty()),
            strict_inbound_webhooks: dto.strict_inbound_webhooks,
            strict_outbound_egress: dto.strict_outbound_egress,
            source: "database".to_owned(),
            updated_at: Utc::now().to_rfc3339(),
        };

        self.store.0.upsert_identity(record).await
    }

    /// Records an inbound external call (webhook or A2A) in the indefinite ledger.
    /// External webhooks without identity headers are allowed unless `strict_inbound_webhooks` is enabled.
    pub async fn record_inbound(
        &self,
        protocol: &str,
        endpoint_path: &str,
        method: &str,
        headers: &HeaderMap,
        task_id: Option<&str>,
        detail: Value,
    ) -> Result<AgentInteractionLedgerEntry, ServerError> {
        let remote_org = header_str(headers, HEADER_AGENT_ORGANIZATION);
        let remote_id = header_str(headers, HEADER_AGENT_INSTALLATION_ID);
        let code_present = header_str(headers, HEADER_AGENT_IDENTITY_CODE).is_some();
        let remote_ip =
            header_str(headers, "x-forwarded-for").or_else(|| header_str(headers, "x-real-ip"));
        let trace_id =
            header_str(headers, "x-trace-id").or_else(|| header_str(headers, "traceparent"));

        let verification_status =
            classify_inbound_identity(remote_org.as_deref(), remote_id.as_deref(), code_present);

        let strict_inbound = self
            .resolve_installation_identity()
            .await?
            .is_some_and(|cfg| cfg.strict_inbound_webhooks);

        let status_code = if strict_inbound && verification_status == "missing" {
            401
        } else {
            200
        };

        let entry = AgentInteractionLedgerEntry {
            id: uuid::Uuid::new_v4().to_string(),
            direction: "inbound".to_owned(),
            protocol: protocol.to_owned(),
            remote_org,
            remote_installation_id: remote_id,
            identity_code_present: code_present,
            verification_status: verification_status.to_owned(),
            remote_ip,
            target_host: None,
            endpoint_path: endpoint_path.to_owned(),
            method: method.to_owned(),
            status_code: Some(status_code),
            trace_id,
            task_id: task_id.map(str::to_owned),
            detail_json: detail,
            created_at: Utc::now().to_rfc3339(),
        };

        self.store.0.append_ledger(entry.clone()).await?;
        info!(
            direction = "inbound",
            protocol = %entry.protocol,
            verification_status = %entry.verification_status,
            remote_org = ?entry.remote_org,
            remote_installation_id = ?entry.remote_installation_id,
            identity_code_present = entry.identity_code_present,
            task_id = ?entry.task_id,
            endpoint = %entry.endpoint_path,
            "recorded inbound agent/webhook interaction"
        );

        if strict_inbound && verification_status == "missing" {
            return Err(ServerError::Unauthorized(
                "installation identity headers required by strict inbound policy".into(),
            ));
        }

        Ok(entry)
    }

    /// Injects mandatory installation identity headers into outbound HTTP requests.
    /// Fails closed when `strict_outbound_egress` is enabled and no identity is configured.
    pub async fn inject_outbound_headers(
        &self,
        headers: &mut HeaderMap,
    ) -> Result<InstallationIdentityRecord, ServerError> {
        let Some(cfg) = self.resolve_installation_identity().await? else {
            return Err(ServerError::Forbidden(
                "installation identity is not configured for outbound requests".into(),
            ));
        };

        if !is_valid_organization_name(&cfg.organization_name)
            || !is_valid_alphanumeric_id(&cfg.installation_id)
        {
            return Err(ServerError::Forbidden(
                "installation identity headers are invalid".into(),
            ));
        }

        set_header(headers, HEADER_AGENT_ORGANIZATION, &cfg.organization_name)?;
        set_header(headers, HEADER_AGENT_INSTALLATION_ID, &cfg.installation_id)?;
        if let Some(code) = &cfg.identity_code {
            set_header(headers, HEADER_AGENT_IDENTITY_CODE, code)?;
        }

        Ok(cfg)
    }

    /// Records an outbound external agent/webhook call in the indefinite ledger.
    pub async fn record_outbound(
        &self,
        rec: OutboundInteractionRecord<'_>,
    ) -> Result<(), ServerError> {
        let entry = AgentInteractionLedgerEntry {
            id: uuid::Uuid::new_v4().to_string(),
            direction: "outbound".to_owned(),
            protocol: rec.protocol.to_owned(),
            remote_org: rec.identity.map(|i| i.organization_name.clone()),
            remote_installation_id: rec.identity.map(|i| i.installation_id.clone()),
            identity_code_present: rec.identity.is_some_and(|i| i.has_identity_code),
            verification_status: if rec.identity.is_some_and(|i| i.has_identity_code) {
                "verified".to_owned()
            } else if rec.identity.is_some() {
                "unverified".to_owned()
            } else {
                "missing".to_owned()
            },
            remote_ip: None,
            target_host: rec.target_host.map(str::to_owned),
            endpoint_path: rec.endpoint_path.to_owned(),
            method: rec.method.to_owned(),
            status_code: rec.status_code,
            trace_id: None,
            task_id: rec.task_id.map(str::to_owned),
            detail_json: json!({}),
            created_at: Utc::now().to_rfc3339(),
        };
        self.store.0.append_ledger(entry.clone()).await?;
        info!(
            direction = "outbound",
            protocol = %entry.protocol,
            verification_status = %entry.verification_status,
            agent_org = ?entry.remote_org,
            agent_installation_id = ?entry.remote_installation_id,
            identity_code_present = entry.identity_code_present,
            target_host = ?entry.target_host,
            endpoint = %entry.endpoint_path,
            status_code = ?entry.status_code,
            task_id = ?entry.task_id,
            "recorded outbound agent/webhook interaction"
        );
        Ok(())
    }

    /// Queries the indefinite agent interaction ledger.
    pub async fn list_ledger(
        &self,
        query: LedgerQueryDto,
    ) -> Result<LedgerListResponse, ServerError> {
        let entries = self.store.0.list_ledger(query).await?;
        Ok(LedgerListResponse {
            count: entries.len(),
            entries,
        })
    }
}

fn header_str(headers: &HeaderMap, name: &str) -> Option<String> {
    headers
        .get(name)
        .and_then(|v| v.to_str().ok())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
}

fn set_header(headers: &mut HeaderMap, name: &str, val: &str) -> Result<(), ServerError> {
    let hdr_name = HeaderName::from_bytes(name.as_bytes())
        .map_err(|e| ServerError::Internal(format!("invalid header name `{name}`: {e}")))?;
    let hdr_val = HeaderValue::from_str(val)
        .map_err(|e| ServerError::Internal(format!("invalid header value for `{name}`: {e}")))?;
    headers.insert(hdr_name, hdr_val);
    Ok(())
}

fn classify_inbound_identity(
    org: Option<&str>,
    id: Option<&str>,
    code_present: bool,
) -> &'static str {
    match (org, id) {
        (Some(o), Some(i)) if is_valid_organization_name(o) && is_valid_alphanumeric_id(i) => {
            if code_present {
                "verified"
            } else {
                "unverified"
            }
        }
        _ => "missing",
    }
}

/// Helper to build a default `IdentityService` from an existing store arc.
#[must_use]
pub fn build_identity_service(
    store: Arc<dyn super::repositories::IdentityRepository>,
) -> IdentityService {
    IdentityService::new(Arc::new(IdentityStore(store)))
}
