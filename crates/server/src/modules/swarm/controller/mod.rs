//! Thin Ironic HTTP controller for multi-agent swarm handoffs, signed messaging,
//! and Valkey shared blackboards.

use std::sync::Arc;

use agents_protocol::ApiKeyScope;
use ironic::prelude::*;
use serde_json::{Value, json};

use crate::{
    infra::error::validate_dto,
    modules::{
        auth::{
            entities::AuthenticatedActor,
            guards::{AuthGuard, CurrentActor, require_scope},
        },
        swarm::{
            dto::{CreateHandoffDto, PutBlackboardDto, SendSwarmMessageDto},
            entities::{MediatedSwarmMessage, SwarmHandoffEntity},
            services::SwarmService,
        },
    },
};

#[controller("/api/v1/sessions")]
#[guard(AuthGuard)]
#[derive(Injectable)]
pub struct SwarmController {
    service: Arc<SwarmService>,
}

#[routes]
impl SwarmController {
    #[post("/:session_id/swarm/handoffs")]
    #[api(
        summary = "Create subagent handoff with narrowed permissions",
        tag = "Swarm",
        security = "bearerAuth"
    )]
    #[body(json = CreateHandoffDto)]
    #[resp(200, "Created subagent handoff", json = SwarmHandoffEntity)]
    #[resp(403, "Permission escalation rejected")]
    async fn create_handoff(
        &self,
        #[decorator(CurrentActor)] actor: AuthenticatedActor,
        #[param] session_id: String,
        #[body] dto: CreateHandoffDto,
    ) -> Result<Json<SwarmHandoffEntity>, HttpError> {
        validate_dto(&dto)?;
        require_scope(&actor, ApiKeyScope::Write)?;
        let saved = self
            .service
            .create_handoff(&actor, &session_id, &dto)
            .await?;
        Ok(Json(saved))
    }

    #[get("/:session_id/swarm/handoffs")]
    #[api(
        summary = "List session swarm handoffs",
        tag = "Swarm",
        security = "bearerAuth"
    )]
    #[resp(200, "Swarm handoffs", json = Vec<SwarmHandoffEntity>)]
    async fn list_handoffs(
        &self,
        #[decorator(CurrentActor)] actor: AuthenticatedActor,
        #[param] session_id: String,
    ) -> Result<Json<Vec<SwarmHandoffEntity>>, HttpError> {
        require_scope(&actor, ApiKeyScope::Read)?;
        let list = self.service.list_handoffs(&actor, &session_id).await?;
        Ok(Json(list))
    }

    #[post("/:session_id/swarm/messages")]
    #[api(
        summary = "Send signed or mediated swarm message",
        tag = "Swarm",
        security = "bearerAuth"
    )]
    #[body(json = SendSwarmMessageDto)]
    #[resp(200, "Dispatched swarm message", json = MediatedSwarmMessage)]
    async fn send_message(
        &self,
        #[decorator(CurrentActor)] actor: AuthenticatedActor,
        #[param] session_id: String,
        #[body] dto: SendSwarmMessageDto,
    ) -> Result<Json<MediatedSwarmMessage>, HttpError> {
        validate_dto(&dto)?;
        require_scope(&actor, ApiKeyScope::Write)?;
        let msg = self.service.send_message(&actor, &session_id, &dto).await?;
        Ok(Json(msg))
    }

    #[get("/:session_id/swarm/messages")]
    #[api(
        summary = "List session swarm messages",
        tag = "Swarm",
        security = "bearerAuth"
    )]
    #[resp(200, "Swarm messages", json = Vec<MediatedSwarmMessage>)]
    async fn list_messages(
        &self,
        #[decorator(CurrentActor)] actor: AuthenticatedActor,
        #[param] session_id: String,
    ) -> Result<Json<Vec<MediatedSwarmMessage>>, HttpError> {
        require_scope(&actor, ApiKeyScope::Read)?;
        let list = self.service.list_messages(&actor, &session_id).await?;
        Ok(Json(list))
    }

    #[post("/:session_id/swarm/blackboard")]
    #[api(
        summary = "Write Valkey shared blackboard key",
        tag = "Swarm",
        security = "bearerAuth"
    )]
    #[body(json = PutBlackboardDto)]
    #[resp(200, "Stored blackboard value")]
    async fn put_blackboard(
        &self,
        #[decorator(CurrentActor)] actor: AuthenticatedActor,
        #[param] session_id: String,
        #[body] dto: PutBlackboardDto,
    ) -> Result<Json<Value>, HttpError> {
        validate_dto(&dto)?;
        require_scope(&actor, ApiKeyScope::Write)?;
        let val = self
            .service
            .put_blackboard(&actor, &session_id, &dto)
            .await?;
        Ok(Json(json!({ "key": dto.key, "value": val })))
    }

    #[get("/:session_id/swarm/blackboard/:key")]
    #[api(
        summary = "Read Valkey shared blackboard key",
        tag = "Swarm",
        security = "bearerAuth"
    )]
    #[resp(200, "Blackboard entry")]
    async fn get_blackboard(
        &self,
        #[decorator(CurrentActor)] actor: AuthenticatedActor,
        #[param] session_id: String,
        #[param] key: String,
    ) -> Result<Json<Value>, HttpError> {
        require_scope(&actor, ApiKeyScope::Read)?;
        let val = self
            .service
            .get_blackboard(&actor, &session_id, &key)
            .await?;
        Ok(Json(json!({ "key": key, "value": val })))
    }
}
