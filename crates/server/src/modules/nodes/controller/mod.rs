//! Thin Ironic controller for VLAN node discovery plus Traefik HTTP provider route.

use std::sync::{Arc, OnceLock};

use agents_protocol::{ApiKeyScope, BoxerNodeAnnouncement};
use axum::{
    Router,
    extract::State,
    http::StatusCode,
    response::{IntoResponse, Json as AxumJson, Response},
    routing::get,
};
use ironic::prelude::*;

use crate::{
    infra::{
        config::ServerConfig,
        error::{ErrorResponse, validate_dto},
    },
    modules::{
        auth::{
            entities::AuthenticatedActor,
            guards::{AuthGuard, CurrentActor, ScopeGuard, require_scope},
        },
        nodes::{
            dto::{
                NodeHeartbeatDto, NodeRegistrationDto, NodeRegistrationResponse, PreviewSyncDto,
            },
            entities::{NodeListResponse, PreviewSyncAck},
            services::NodeRegistryService,
        },
    },
};

/// Shared handle exposing dynamic Traefik HTTP provider routes to the router layer.
#[derive(Clone)]
pub struct NodeHttpState {
    pub service: Arc<NodeRegistryService>,
}

#[controller("/api/v1/nodes")]
#[derive(Injectable)]
pub struct NodesController {
    service: Arc<NodeRegistryService>,
    config: Arc<ServerConfig>,
}

#[routes]
impl NodesController {
    #[get]
    #[guard(AuthGuard)]
    #[guard(ScopeGuard::new(ApiKeyScope::Read))]
    #[api(
        summary = "List active auto-registered private-VLAN Boxer nodes",
        tag = "Nodes",
        security = "bearerAuth"
    )]
    #[resp(200, "Active worker nodes", json = NodeListResponse)]
    #[resp(401, "Unauthorized", json = ErrorResponse)]
    #[resp(403, "Forbidden", json = ErrorResponse)]
    #[resp(500, "Internal server error", json = ErrorResponse)]
    async fn list(
        &self,
        #[decorator(CurrentActor)] actor: AuthenticatedActor,
    ) -> Result<Json<NodeListResponse>, HttpError> {
        require_scope(&actor, ApiKeyScope::Read)?;
        Ok(Json(self.service.list_active_nodes().await))
    }

    #[post("/heartbeat")]
    #[guard(AuthGuard)]
    #[guard(ScopeGuard::new(ApiKeyScope::Exec))]
    #[api(
        summary = "Register or refresh a private-VLAN Boxer worker node",
        tag = "Nodes",
        security = "bearerAuth"
    )]
    #[body(json = NodeHeartbeatDto)]
    #[resp(200, "Registered node snapshot", json = BoxerNodeAnnouncement)]
    #[resp(401, "Unauthorized", json = ErrorResponse)]
    #[resp(403, "Forbidden", json = ErrorResponse)]
    #[resp(422, "Validation error", json = ErrorResponse)]
    #[resp(500, "Internal server error", json = ErrorResponse)]
    async fn heartbeat(
        &self,
        #[decorator(CurrentActor)] actor: AuthenticatedActor,
        #[body] dto: NodeHeartbeatDto,
    ) -> Result<Json<BoxerNodeAnnouncement>, HttpError> {
        validate_dto(&dto)?;
        require_scope(&actor, ApiKeyScope::Exec)?;
        let node = self.service.register_heartbeat(&actor, &dto).await?;
        Ok(Json(node))
    }

    #[post("/register")]
    #[api(
        summary = "Bootstrap and authenticate Boxer worker node via mutual HMAC-SHA256 handshake",
        tag = "Nodes"
    )]
    #[body(json = NodeRegistrationDto)]
    #[resp(200, "Mutual registration acknowledgment", json = NodeRegistrationResponse)]
    #[resp(400, "Bad Request", json = ErrorResponse)]
    #[resp(422, "Validation error", json = ErrorResponse)]
    #[resp(500, "Internal server error", json = ErrorResponse)]
    async fn register(
        &self,
        #[body] dto: NodeRegistrationDto,
    ) -> Result<Json<NodeRegistrationResponse>, HttpError> {
        validate_dto(&dto)?;
        let res = self
            .service
            .register_node_hmac(&dto, &self.config.cluster_secret)
            .await?;
        Ok(Json(res))
    }

    #[post("/previews")]
    #[guard(AuthGuard)]
    #[guard(ScopeGuard::new(ApiKeyScope::Exec))]
    #[api(
        summary = "Publish or revoke a distributed Traefik preview route",
        tag = "Nodes",
        security = "bearerAuth"
    )]
    #[body(json = PreviewSyncDto)]
    #[resp(200, "Preview sync acknowledgment", json = PreviewSyncAck)]
    #[resp(401, "Unauthorized", json = ErrorResponse)]
    #[resp(403, "Forbidden", json = ErrorResponse)]
    #[resp(422, "Validation error", json = ErrorResponse)]
    #[resp(500, "Internal server error", json = ErrorResponse)]
    async fn sync_preview(
        &self,
        #[decorator(CurrentActor)] actor: AuthenticatedActor,
        #[body] dto: PreviewSyncDto,
    ) -> Result<Json<PreviewSyncAck>, HttpError> {
        validate_dto(&dto)?;
        require_scope(&actor, ApiKeyScope::Exec)?;
        let ack = self.service.sync_preview(&actor, &dto).await?;
        Ok(Json(ack))
    }
}

/// Mounts the local Traefik HTTP dynamic provider endpoint cleanly with lazy DI wiring.
pub fn traefik_lazy_router(cell: Arc<OnceLock<NodeHttpState>>) -> Router {
    Router::new()
        .route("/internal/traefik/dynamic", get(traefik_dynamic_lazy))
        .with_state(cell)
}

async fn traefik_dynamic_lazy(State(cell): State<Arc<OnceLock<NodeHttpState>>>) -> Response {
    let Some(state) = cell.get() else {
        return (StatusCode::SERVICE_UNAVAILABLE, "nodes initializing").into_response();
    };
    let doc = state.service.render_traefik_http_config().await;
    AxumJson(doc).into_response()
}
