//! Thin Ironic HTTP controllers for A2A Agent Card discovery, JSON-RPC postal intake, and outbound A2A calls.

use std::sync::Arc;

use a2a::{AgentCard, JsonRpcRequest, JsonRpcResponse};
use agents_protocol::ApiKeyScope;
use garde::Validate;
use ironic::prelude::*;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::{
    infra::{
        error::{ErrorResponse, ServerError, validate_dto},
        observability::PerRouteTimeoutInterceptor,
    },
    modules::{
        auth::guards::{AuthGuard, ScopeGuard},
        identity::entities::{InboundIdentityContext, InboundIdentityExtractor},
    },
};

use super::services::{A2aService, OutboundA2aResponse, OutboundA2aSendDto};

/// Public A2A discovery endpoint (`GET /.well-known/agent-card.json`).
#[controller("/.well-known")]
#[derive(Injectable)]
pub struct A2aDiscoveryController {
    service: Arc<A2aService>,
}

#[routes]
impl A2aDiscoveryController {
    #[get("/agent-card.json")]
    #[interceptor(PerRouteTimeoutInterceptor::secs(5))]
    #[api(summary = "Get A2A Agent Card", tag = "A2A")]
    #[resp(200, "Official A2A v1 AgentCard")]
    async fn get_agent_card(
        &self,
        #[decorator(InboundIdentityExtractor)] inbound: InboundIdentityContext,
    ) -> Result<Json<AgentCard>, HttpError> {
        let base_url = format!("{}://{}", inbound.proto, inbound.host);
        Ok(Json(self.service.build_agent_card(&base_url).await))
    }
}

/// Wrapper DTO for A2A JSON-RPC 2.0 request bodies.
#[derive(Clone, Debug, Deserialize, Validate)]
pub struct A2aRpcPayloadDto {
    #[garde(length(min = 3, max = 16))]
    pub jsonrpc: String,
    #[garde(skip)]
    pub id: Value,
    #[garde(length(min = 1, max = 128))]
    pub method: String,
    #[garde(skip)]
    pub params: Option<Value>,
}

impl OpenApiSchema for A2aRpcPayloadDto {
    fn openapi_schema() -> Value {
        json!({
            "type": "object",
            "properties": {
                "jsonrpc": { "type": "string", "example": "2.0" },
                "id": { "type": ["string", "integer"] },
                "method": { "type": "string", "example": "message/send" },
                "params": { "type": "object" }
            },
            "required": ["jsonrpc", "id", "method"]
        })
    }
}

/// Public A2A JSON-RPC 2.0 postal gateway (`POST /a2a`).
#[controller("/a2a")]
#[derive(Injectable)]
pub struct A2aRpcController {
    service: Arc<A2aService>,
}

#[routes]
impl A2aRpcController {
    #[post("/")]
    #[interceptor(PerRouteTimeoutInterceptor::secs(30))]
    #[api(summary = "Invoke A2A JSON-RPC 2.0 postal gateway", tag = "A2A")]
    #[body(json = A2aRpcPayloadDto)]
    #[resp(200, "A2A JSON-RPC 2.0 response")]
    #[resp(400, "Invalid A2A JSON-RPC payload", json = ErrorResponse)]
    async fn handle_rpc(
        &self,
        #[decorator(InboundIdentityExtractor)] inbound: InboundIdentityContext,
        #[body] dto: A2aRpcPayloadDto,
    ) -> Result<Json<JsonRpcResponse>, HttpError> {
        let raw = json!({
            "jsonrpc": dto.jsonrpc,
            "id": dto.id,
            "method": dto.method,
            "params": dto.params,
        });
        let req: JsonRpcRequest = serde_json::from_value(raw)
            .map_err(|e| ServerError::Validation(format!("invalid A2A JSON-RPC payload: {e}")))?;
        Ok(Json(
            self.service.handle_jsonrpc(req, &inbound.headers).await,
        ))
    }
}

/// Authenticated orchestrator outbound A2A client (`POST /api/v1/a2a/send`).
#[controller("/api/v1/a2a")]
#[guard(AuthGuard)]
#[derive(Injectable)]
pub struct A2aOutboundController {
    service: Arc<A2aService>,
}

#[routes]
impl A2aOutboundController {
    #[post("/send")]
    #[guard(ScopeGuard::new(ApiKeyScope::Write))]
    #[interceptor(PerRouteTimeoutInterceptor::secs(30))]
    #[api(
        summary = "Send outbound A2A message to remote agent",
        tag = "A2A",
        security = "bearerAuth"
    )]
    #[body(json = OutboundA2aSendDto)]
    #[resp(200, "Outbound A2A response", json = OutboundA2aResponse)]
    #[resp(403, "Missing installation identity or insufficient scope", json = ErrorResponse)]
    async fn send_outbound(
        &self,
        #[body] dto: OutboundA2aSendDto,
    ) -> Result<Json<OutboundA2aResponse>, HttpError> {
        validate_dto(&dto)?;
        let res = self.service.send_outbound(&dto).await?;
        Ok(Json(res))
    }
}
