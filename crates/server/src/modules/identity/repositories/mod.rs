//! Repository abstraction for singleton installation identity and indefinite interaction ledger.

use std::{future::Future, pin::Pin, sync::Arc};

use sqlx::{PgPool, Row};
use tokio::sync::RwLock;

use crate::infra::error::ServerError;

use super::entities::{AgentInteractionLedgerEntry, InstallationIdentityRecord, LedgerQueryDto};

pub type RepoFuture<'a, T> = Pin<Box<dyn Future<Output = Result<T, ServerError>> + Send + 'a>>;

/// Persistence port for installation identity and indefinite interaction ledger.
pub trait IdentityRepository: Send + Sync {
    fn get_identity(&self) -> RepoFuture<'_, Option<InstallationIdentityRecord>>;
    fn upsert_identity(
        &self,
        record: InstallationIdentityRecord,
    ) -> RepoFuture<'_, InstallationIdentityRecord>;
    fn append_ledger(&self, entry: AgentInteractionLedgerEntry) -> RepoFuture<'_, ()>;
    fn list_ledger(
        &self,
        query: LedgerQueryDto,
    ) -> RepoFuture<'_, Vec<AgentInteractionLedgerEntry>>;
}

/// Shared injectable store wrapper.
#[derive(Clone)]
pub struct IdentityStore(pub Arc<dyn IdentityRepository>);

impl Default for IdentityStore {
    fn default() -> Self {
        Self(Arc::new(InMemoryIdentityRepository::default()))
    }
}

/// In-memory repository for unit and integration tests.
#[derive(Default)]
pub struct InMemoryIdentityRepository {
    identity: RwLock<Option<InstallationIdentityRecord>>,
    ledger: RwLock<Vec<AgentInteractionLedgerEntry>>,
}

impl IdentityRepository for InMemoryIdentityRepository {
    fn get_identity(&self) -> RepoFuture<'_, Option<InstallationIdentityRecord>> {
        Box::pin(async move { Ok(self.identity.read().await.clone()) })
    }

    fn upsert_identity(
        &self,
        record: InstallationIdentityRecord,
    ) -> RepoFuture<'_, InstallationIdentityRecord> {
        Box::pin(async move {
            *self.identity.write().await = Some(record.clone());
            Ok(record)
        })
    }

    fn append_ledger(&self, entry: AgentInteractionLedgerEntry) -> RepoFuture<'_, ()> {
        Box::pin(async move {
            self.ledger.write().await.push(entry);
            Ok(())
        })
    }

    fn list_ledger(
        &self,
        query: LedgerQueryDto,
    ) -> RepoFuture<'_, Vec<AgentInteractionLedgerEntry>> {
        Box::pin(async move {
            let limit = query.limit.unwrap_or(100).clamp(1, 500);
            let items = self.ledger.read().await;
            let filtered: Vec<AgentInteractionLedgerEntry> = items
                .iter()
                .rev()
                .filter(|e| {
                    query
                        .direction
                        .as_deref()
                        .is_none_or(|d| e.direction.eq_ignore_ascii_case(d))
                })
                .filter(|e| {
                    query
                        .protocol
                        .as_deref()
                        .is_none_or(|p| e.protocol.eq_ignore_ascii_case(p))
                })
                .filter(|e| {
                    query
                        .remote_org
                        .as_deref()
                        .is_none_or(|o| e.remote_org.as_deref() == Some(o))
                })
                .filter(|e| {
                    query
                        .verification_status
                        .as_deref()
                        .is_none_or(|v| e.verification_status.eq_ignore_ascii_case(v))
                })
                .take(limit)
                .cloned()
                .collect();
            Ok(filtered)
        })
    }
}

/// PostgreSQL-backed repository providing durable singleton identity and indefinite ledger storage.
pub struct PgIdentityRepository {
    pool: PgPool,
}

impl PgIdentityRepository {
    #[must_use]
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

impl IdentityRepository for PgIdentityRepository {
    fn get_identity(&self) -> RepoFuture<'_, Option<InstallationIdentityRecord>> {
        Box::pin(async move {
            let row = sqlx::query(
                "SELECT organization_name, installation_id, identity_code, registrar_url, \
                 strict_inbound_webhooks, strict_outbound_egress, \
                 to_char(updated_at AT TIME ZONE 'UTC', 'YYYY-MM-DD\"T\"HH24:MI:SS\"Z\"') AS updated_at_str \
                 FROM installation_identity WHERE singleton_lock = TRUE",
            )
            .fetch_optional(&self.pool)
            .await
            .map_err(|e| ServerError::Internal(format!("identity fetch failed: {e}")))?;

            Ok(row.map(|r| {
                let code: Option<String> = r.get("identity_code");
                let has_code = code.as_deref().is_some_and(|c| !c.trim().is_empty());
                InstallationIdentityRecord {
                    organization_name: r.get("organization_name"),
                    installation_id: r.get("installation_id"),
                    identity_code: code,
                    has_identity_code: has_code,
                    registrar_url: r.get("registrar_url"),
                    strict_inbound_webhooks: r.get("strict_inbound_webhooks"),
                    strict_outbound_egress: r.get("strict_outbound_egress"),
                    source: "database".to_owned(),
                    updated_at: r.get("updated_at_str"),
                }
            }))
        })
    }

    fn upsert_identity(
        &self,
        record: InstallationIdentityRecord,
    ) -> RepoFuture<'_, InstallationIdentityRecord> {
        Box::pin(async move {
            sqlx::query(
                "INSERT INTO installation_identity \
                 (singleton_lock, organization_name, installation_id, identity_code, registrar_url, \
                  strict_inbound_webhooks, strict_outbound_egress, updated_at) \
                 VALUES (TRUE, $1, $2, $3, $4, $5, $6, NOW()) \
                 ON CONFLICT (singleton_lock) DO UPDATE SET \
                 organization_name = EXCLUDED.organization_name, \
                 installation_id = EXCLUDED.installation_id, \
                 identity_code = COALESCE(EXCLUDED.identity_code, installation_identity.identity_code), \
                 registrar_url = EXCLUDED.registrar_url, \
                 strict_inbound_webhooks = EXCLUDED.strict_inbound_webhooks, \
                 strict_outbound_egress = EXCLUDED.strict_outbound_egress, \
                 updated_at = NOW()",
            )
            .bind(&record.organization_name)
            .bind(&record.installation_id)
            .bind(&record.identity_code)
            .bind(&record.registrar_url)
            .bind(record.strict_inbound_webhooks)
            .bind(record.strict_outbound_egress)
            .execute(&self.pool)
            .await
            .map_err(|e| ServerError::Internal(format!("identity upsert failed: {e}")))?;

            Ok(record)
        })
    }

    fn append_ledger(&self, entry: AgentInteractionLedgerEntry) -> RepoFuture<'_, ()> {
        Box::pin(async move {
            let uuid_str = uuid::Uuid::parse_str(&entry.id)
                .map(|u| u.to_string())
                .unwrap_or_else(|_| uuid::Uuid::new_v4().to_string());
            sqlx::query(
                "INSERT INTO agent_interaction_ledger \
                 (id, direction, protocol, remote_org, remote_installation_id, \
                  identity_code_present, verification_status, remote_ip, target_host, \
                  endpoint_path, method, status_code, trace_id, task_id, detail_json) \
                 VALUES ($1::uuid, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15)",
            )
            .bind(uuid_str)
            .bind(&entry.direction)
            .bind(&entry.protocol)
            .bind(&entry.remote_org)
            .bind(&entry.remote_installation_id)
            .bind(entry.identity_code_present)
            .bind(&entry.verification_status)
            .bind(&entry.remote_ip)
            .bind(&entry.target_host)
            .bind(&entry.endpoint_path)
            .bind(&entry.method)
            .bind(entry.status_code)
            .bind(&entry.trace_id)
            .bind(&entry.task_id)
            .bind(&entry.detail_json)
            .execute(&self.pool)
            .await
            .map_err(|e| ServerError::Internal(format!("ledger append failed: {e}")))?;

            Ok(())
        })
    }

    fn list_ledger(
        &self,
        query: LedgerQueryDto,
    ) -> RepoFuture<'_, Vec<AgentInteractionLedgerEntry>> {
        Box::pin(async move {
            let limit = i64::try_from(query.limit.unwrap_or(100).clamp(1, 500)).unwrap_or(100);
            let rows = sqlx::query(
                "SELECT id::text AS id_str, direction, protocol, remote_org, remote_installation_id, \
                 identity_code_present, verification_status, remote_ip, target_host, \
                 endpoint_path, method, status_code, trace_id, task_id, detail_json, \
                 to_char(created_at AT TIME ZONE 'UTC', 'YYYY-MM-DD\"T\"HH24:MI:SS\"Z\"') AS created_at_str \
                 FROM agent_interaction_ledger \
                 WHERE ($1::text IS NULL OR LOWER(direction) = LOWER($1)) \
                 AND ($2::text IS NULL OR LOWER(protocol) = LOWER($2)) \
                 AND ($3::text IS NULL OR remote_org = $3) \
                 AND ($4::text IS NULL OR LOWER(verification_status) = LOWER($4)) \
                 ORDER BY created_at DESC LIMIT $5",
            )
            .bind(&query.direction)
            .bind(&query.protocol)
            .bind(&query.remote_org)
            .bind(&query.verification_status)
            .bind(limit)
            .fetch_all(&self.pool)
            .await
            .map_err(|e| ServerError::Internal(format!("ledger query failed: {e}")))?;

            Ok(rows
                .into_iter()
                .map(|r| AgentInteractionLedgerEntry {
                    id: r.get("id_str"),
                    direction: r.get("direction"),
                    protocol: r.get("protocol"),
                    remote_org: r.get("remote_org"),
                    remote_installation_id: r.get("remote_installation_id"),
                    identity_code_present: r.get("identity_code_present"),
                    verification_status: r.get("verification_status"),
                    remote_ip: r.get("remote_ip"),
                    target_host: r.get("target_host"),
                    endpoint_path: r.get("endpoint_path"),
                    method: r.get("method"),
                    status_code: r.get("status_code"),
                    trace_id: r.get("trace_id"),
                    task_id: r.get("task_id"),
                    detail_json: r.get("detail_json"),
                    created_at: r.get("created_at_str"),
                })
                .collect())
        })
    }
}
