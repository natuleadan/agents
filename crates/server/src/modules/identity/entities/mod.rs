//! Domain entities and DTOs for installation identity and the indefinite interaction ledger.

use agents_protocol::{
    HEADER_AGENT_IDENTITY_CODE, HEADER_AGENT_INSTALLATION_ID, HEADER_AGENT_ORGANIZATION,
};
use axum::http::{HeaderMap, HeaderName, HeaderValue};
use garde::Validate;
use ironic::{ExtractFuture, ExtractedValue, OpenApiSchema, ParameterExtractor, RequestContext};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

/// Singleton installation identity configuration (resolved from `.env` with DB fallback).
#[derive(Clone, Debug, Serialize, Deserialize, OpenApiSchema)]
#[serde(rename_all = "camelCase")]
pub struct InstallationIdentityRecord {
    pub organization_name: String,
    pub installation_id: String,
    #[serde(skip_serializing)]
    pub identity_code: Option<String>,
    pub has_identity_code: bool,
    pub registrar_url: Option<String>,
    pub strict_inbound_webhooks: bool,
    pub strict_outbound_egress: bool,
    pub source: String,
    pub updated_at: String,
}

/// Request payload to update the singleton installation identity in the database.
#[derive(Clone, Debug, Deserialize, Validate, OpenApiSchema)]
#[serde(rename_all = "camelCase")]
pub struct UpdateInstallationIdentityDto {
    #[garde(length(min = 2, max = 128))]
    pub organization_name: String,
    #[garde(length(min = 4, max = 64))]
    pub installation_id: String,
    #[garde(length(max = 4096))]
    pub identity_code: Option<String>,
    #[garde(length(max = 512))]
    pub registrar_url: Option<String>,
    #[serde(default)]
    #[garde(skip)]
    pub strict_inbound_webhooks: bool,
    #[serde(default)]
    #[garde(skip)]
    pub strict_outbound_egress: bool,
}

/// Permanent ledger record for every inbound and outbound external agent/webhook interaction.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentInteractionLedgerEntry {
    pub id: String,
    pub direction: String,
    pub protocol: String,
    pub remote_org: Option<String>,
    pub remote_installation_id: Option<String>,
    pub identity_code_present: bool,
    pub verification_status: String,
    pub remote_ip: Option<String>,
    pub target_host: Option<String>,
    pub endpoint_path: String,
    pub method: String,
    pub status_code: Option<i32>,
    pub trace_id: Option<String>,
    pub task_id: Option<String>,
    pub detail_json: Value,
    pub created_at: String,
}

impl OpenApiSchema for AgentInteractionLedgerEntry {
    fn openapi_schema() -> Value {
        json!({
            "type": "object",
            "properties": {
                "id": { "type": "string" },
                "direction": { "type": "string", "enum": ["inbound", "outbound"] },
                "protocol": { "type": "string" },
                "remoteOrg": { "type": ["string", "null"] },
                "remoteInstallationId": { "type": ["string", "null"] },
                "identityCodePresent": { "type": "boolean" },
                "verificationStatus": { "type": "string", "enum": ["verified", "unverified", "missing"] },
                "remoteIp": { "type": ["string", "null"] },
                "targetHost": { "type": ["string", "null"] },
                "endpointPath": { "type": "string" },
                "method": { "type": "string" },
                "statusCode": { "type": ["integer", "null"] },
                "traceId": { "type": ["string", "null"] },
                "taskId": { "type": ["string", "null"] },
                "detailJson": { "type": "object" },
                "createdAt": { "type": "string" }
            },
            "required": ["id", "direction", "protocol", "identityCodePresent", "verificationStatus", "endpointPath", "method", "createdAt"]
        })
    }
}

/// Query filters for inspecting the permanent agent interaction ledger.
#[derive(Clone, Debug, Default, Deserialize, Validate)]
pub struct LedgerQueryDto {
    #[garde(length(max = 16))]
    pub direction: Option<String>,
    #[garde(length(max = 32))]
    pub protocol: Option<String>,
    #[garde(length(max = 128))]
    pub remote_org: Option<String>,
    #[garde(length(max = 32))]
    pub verification_status: Option<String>,
    #[garde(range(min = 1, max = 500))]
    pub limit: Option<usize>,
}

/// Paginated response for the indefinite agent interaction ledger.
#[derive(Clone, Debug, Serialize, Deserialize, OpenApiSchema)]
#[serde(rename_all = "camelCase")]
pub struct LedgerListResponse {
    pub count: usize,
    pub entries: Vec<AgentInteractionLedgerEntry>,
}

/// Extracted inbound identity, proxy, and tracing headers from an HTTP request.
#[derive(Clone, Debug, Default)]
pub struct InboundIdentityContext {
    pub headers: HeaderMap,
    pub proto: String,
    pub host: String,
}

/// Ironic `#[decorator(InboundIdentityExtractor)]` capturing inbound identity headers.
#[derive(Default)]
pub struct InboundIdentityExtractor;

impl InboundIdentityExtractor {
    #[must_use]
    pub const fn new() -> Self {
        Self
    }
}

impl ParameterExtractor for InboundIdentityExtractor {
    fn extract<'a>(&'a self, context: &'a mut RequestContext) -> ExtractFuture<'a> {
        Box::pin(async move {
            let mut headers = HeaderMap::new();
            let req_headers = context.request().headers();
            for name in [
                HEADER_AGENT_ORGANIZATION,
                HEADER_AGENT_INSTALLATION_ID,
                HEADER_AGENT_IDENTITY_CODE,
                "x-forwarded-for",
                "x-real-ip",
                "x-trace-id",
                "traceparent",
            ] {
                if let Some(val) = req_headers.get(name).and_then(|v| v.to_str().ok())
                    && let (Ok(hn), Ok(hv)) = (
                        HeaderName::from_bytes(name.as_bytes()),
                        HeaderValue::from_str(val),
                    )
                {
                    headers.insert(hn, hv);
                }
            }
            let proto = req_headers
                .get("x-forwarded-proto")
                .and_then(|v| v.to_str().ok())
                .unwrap_or("https")
                .to_owned();
            let host = req_headers
                .get("host")
                .and_then(|v| v.to_str().ok())
                .unwrap_or("127.0.0.1:8080")
                .to_owned();
            let extracted = InboundIdentityContext {
                headers,
                proto,
                host,
            };
            Ok(Box::new(extracted) as ExtractedValue)
        })
    }

    fn description(&self) -> &'static str {
        "inbound installation identity headers"
    }
}
