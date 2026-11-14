//! MCP HTTP controller (policy metadata) and protected Axum middleware for `POST /mcp`.

use std::sync::Arc;

use agents_protocol::ApiKeyScope;
use axum::{
    Json as AxumJson, Router,
    body::{Body, to_bytes},
    extract::{Request as AxumRequest, State},
    http::{HeaderMap, StatusCode},
    middleware::{self, Next},
    response::{IntoResponse, Response},
};
use ironic::prelude::*;
use serde_json::{Value, json};

use crate::{
    infra::error::ErrorResponse,
    modules::{
        auth::{
            entities::AuthenticatedActor,
            guards::{AuthGuard, CurrentActor, ScopeGuard, extract_credential, require_scope},
            services::AuthService,
        },
        mcp::{
            dto::McpPolicyInfo,
            services::{McpHostService, with_mcp_actor},
        },
    },
};

/// Shared dependencies injected into the protected MCP Axum middleware.
#[derive(Clone)]
pub struct McpHttpState {
    pub auth: Arc<AuthService>,
    pub mcp: Arc<McpHostService>,
}

#[controller("/api/v1/mcp")]
#[guard(AuthGuard)]
#[derive(Injectable)]
pub struct McpController {
    service: Arc<McpHostService>,
}

#[routes]
impl McpController {
    #[get("/policy")]
    #[guard(ScopeGuard::new(ApiKeyScope::Read))]
    #[api(
        summary = "Inspect allowlisted MCP tools",
        tag = "MCP",
        security = "bearerAuth"
    )]
    #[resp(200, "MCP tool allowlist policy", json = McpPolicyInfo)]
    #[resp(401, "Unauthorized", json = ErrorResponse)]
    #[resp(403, "Forbidden", json = ErrorResponse)]
    #[resp(500, "Internal server error", json = ErrorResponse)]
    async fn policy(
        &self,
        #[decorator(CurrentActor)] actor: AuthenticatedActor,
    ) -> Result<Json<McpPolicyInfo>, HttpError> {
        require_scope(&actor, ApiKeyScope::Read)?;
        Ok(Json(self.service.policy_info()))
    }
}

/// Builds a protected Axum router mounting `POST /mcp` with Bearer auth, rate limits,
/// and per-tool allowlist enforcement.
pub fn protected_mcp_router(state: McpHttpState) -> Router {
    let base_mcp_router = state.mcp.build_router().build().into_router();
    base_mcp_router.layer(middleware::from_fn_with_state(state, mcp_security_layer))
}

async fn mcp_security_layer(
    State(state): State<McpHttpState>,
    req: AxumRequest,
    next: Next,
) -> Response {
    let headers: &HeaderMap = req.headers();
    let raw_token = match extract_credential(headers) {
        Ok(tok) => tok,
        Err(err) => {
            return (
                StatusCode::UNAUTHORIZED,
                AxumJson(json!({ "error": err.message() })),
            )
                .into_response();
        }
    };
    let actor = match state.auth.authenticate_token(&raw_token).await {
        Ok(actor) => actor,
        Err(err) => {
            return (
                StatusCode::UNAUTHORIZED,
                AxumJson(json!({ "error": err.to_string() })),
            )
                .into_response();
        }
    };

    if let Err(err) = state.mcp.check_rate_limit(&actor).await {
        return (
            StatusCode::TOO_MANY_REQUESTS,
            AxumJson(json!({ "error": err.to_string() })),
        )
            .into_response();
    }

    let (parts, body) = req.into_parts();
    let bytes = match to_bytes(body, 1024 * 1024).await {
        Ok(bytes) => bytes,
        Err(_) => {
            return (
                StatusCode::PAYLOAD_TOO_LARGE,
                AxumJson(json!({ "error": "MCP payload too large" })),
            )
                .into_response();
        }
    };

    if let Ok(parsed) = serde_json::from_slice::<Value>(&bytes)
        && parsed.get("method").and_then(Value::as_str) == Some("tools/call")
        && let Some(tool_name) = parsed
            .get("params")
            .and_then(|p| p.get("name"))
            .and_then(Value::as_str)
        && let Err(err) = state.mcp.authorize_tool_call(&actor, tool_name).await
    {
        return (
            StatusCode::FORBIDDEN,
            AxumJson(json!({
                "jsonrpc": "2.0",
                "id": parsed.get("id").cloned().unwrap_or(Value::Null),
                "error": { "code": -32003, "message": err.to_string() }
            })),
        )
            .into_response();
    }

    let rebuilt = AxumRequest::from_parts(parts, Body::from(bytes));
    with_mcp_actor(actor, next.run(rebuilt)).await
}
