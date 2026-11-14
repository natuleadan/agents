//! Thin Ironic HTTP controller for Autocron schedules and webhook triggers.

use std::sync::Arc;

use agents_protocol::ApiKeyScope;
use ironic::prelude::*;

use crate::{
    infra::{
        error::{ErrorResponse, validate_dto},
        observability::PerRouteTimeoutInterceptor,
    },
    modules::{
        auth::{
            entities::AuthenticatedActor,
            guards::{AuthGuard, CurrentActor, ScopeGuard, require_scope},
        },
        autocron::{
            dto::{CreateScheduleDto, ExecuteScheduleDto, ToggleScheduleDto, TriggerWebhookDto},
            entities::{AutocronScheduleEntity, AutocronTickOutcome},
            services::AutocronService,
        },
    },
};

#[controller("/api/v1/autocron")]
#[guard(AuthGuard)]
#[derive(Injectable)]
pub struct AutocronController {
    service: Arc<AutocronService>,
}

#[routes]
impl AutocronController {
    #[post("/schedules")]
    #[guard(ScopeGuard::new(ApiKeyScope::Write))]
    #[api(
        summary = "Create an Autocron workflow schedule",
        tag = "Autocron",
        security = "bearerAuth"
    )]
    #[body(json = CreateScheduleDto)]
    #[resp(200, "Created Autocron schedule", json = AutocronScheduleEntity)]
    #[resp(401, "Unauthorized", json = ErrorResponse)]
    #[resp(403, "Forbidden", json = ErrorResponse)]
    #[resp(422, "Validation error", json = ErrorResponse)]
    #[resp(500, "Internal server error", json = ErrorResponse)]
    async fn create_schedule(
        &self,
        #[decorator(CurrentActor)] actor: AuthenticatedActor,
        #[body] dto: CreateScheduleDto,
    ) -> Result<Json<AutocronScheduleEntity>, HttpError> {
        validate_dto(&dto)?;
        require_scope(&actor, ApiKeyScope::Write)?;
        let saved = self.service.create_schedule(&actor, &dto).await?;
        Ok(Json(saved))
    }

    #[get("/schedules")]
    #[guard(ScopeGuard::new(ApiKeyScope::Read))]
    #[api(
        summary = "List owned Autocron schedules",
        tag = "Autocron",
        security = "bearerAuth"
    )]
    #[resp(200, "Owned Autocron schedules", json = Vec<AutocronScheduleEntity>)]
    #[resp(401, "Unauthorized", json = ErrorResponse)]
    #[resp(403, "Forbidden", json = ErrorResponse)]
    #[resp(500, "Internal server error", json = ErrorResponse)]
    async fn list_schedules(
        &self,
        #[decorator(CurrentActor)] actor: AuthenticatedActor,
    ) -> Result<Json<Vec<AutocronScheduleEntity>>, HttpError> {
        require_scope(&actor, ApiKeyScope::Read)?;
        let list = self.service.list_owned(&actor).await?;
        Ok(Json(list))
    }

    #[post("/schedules/:schedule_id/run")]
    #[guard(ScopeGuard::new(ApiKeyScope::Exec))]
    #[interceptor(PerRouteTimeoutInterceptor::secs(120))]
    #[api(
        summary = "Execute an Autocron schedule with budget enforcement",
        tag = "Autocron",
        security = "bearerAuth"
    )]
    #[body(json = ExecuteScheduleDto)]
    #[resp(200, "Triggered workflow run outcome", json = AutocronTickOutcome)]
    #[resp(401, "Unauthorized", json = ErrorResponse)]
    #[resp(403, "Forbidden", json = ErrorResponse)]
    #[resp(404, "Schedule not found", json = ErrorResponse)]
    #[resp(409, "Budget cap exhausted or schedule disabled", json = ErrorResponse)]
    #[resp(422, "Validation error", json = ErrorResponse)]
    #[resp(500, "Internal server error", json = ErrorResponse)]
    async fn run_schedule(
        &self,
        #[decorator(CurrentActor)] actor: AuthenticatedActor,
        #[param] schedule_id: String,
        #[body] dto: ExecuteScheduleDto,
    ) -> Result<Json<AutocronTickOutcome>, HttpError> {
        validate_dto(&dto)?;
        require_scope(&actor, ApiKeyScope::Exec)?;
        let outcome = self
            .service
            .execute_owned(&actor, &schedule_id, &dto)
            .await?;
        Ok(Json(outcome))
    }

    #[post("/schedules/:schedule_id/toggle")]
    #[guard(ScopeGuard::new(ApiKeyScope::Write))]
    #[api(
        summary = "Enable or disable an Autocron schedule",
        tag = "Autocron",
        security = "bearerAuth"
    )]
    #[body(json = ToggleScheduleDto)]
    #[resp(200, "Updated Autocron schedule", json = AutocronScheduleEntity)]
    #[resp(401, "Unauthorized", json = ErrorResponse)]
    #[resp(403, "Forbidden", json = ErrorResponse)]
    #[resp(404, "Schedule not found", json = ErrorResponse)]
    #[resp(422, "Validation error", json = ErrorResponse)]
    #[resp(500, "Internal server error", json = ErrorResponse)]
    async fn toggle_schedule(
        &self,
        #[decorator(CurrentActor)] actor: AuthenticatedActor,
        #[param] schedule_id: String,
        #[body] dto: ToggleScheduleDto,
    ) -> Result<Json<AutocronScheduleEntity>, HttpError> {
        validate_dto(&dto)?;
        require_scope(&actor, ApiKeyScope::Write)?;
        let updated = self
            .service
            .toggle_owned(&actor, &schedule_id, dto.enabled)
            .await?;
        Ok(Json(updated))
    }

    #[post("/webhooks/:schedule_id")]
    #[guard(ScopeGuard::new(ApiKeyScope::Exec))]
    #[interceptor(PerRouteTimeoutInterceptor::secs(120))]
    #[api(
        summary = "Invoke a webhook-triggered Autocron schedule",
        tag = "Autocron",
        security = "bearerAuth"
    )]
    #[body(json = TriggerWebhookDto)]
    #[resp(200, "Webhook run dispatched", json = AutocronTickOutcome)]
    #[resp(401, "Unauthorized", json = ErrorResponse)]
    #[resp(403, "Forbidden", json = ErrorResponse)]
    #[resp(404, "Schedule not found", json = ErrorResponse)]
    #[resp(422, "Validation error", json = ErrorResponse)]
    #[resp(500, "Internal server error", json = ErrorResponse)]
    async fn trigger_webhook(
        &self,
        #[decorator(CurrentActor)] actor: AuthenticatedActor,
        #[param] schedule_id: String,
        #[body] dto: TriggerWebhookDto,
    ) -> Result<Json<AutocronTickOutcome>, HttpError> {
        validate_dto(&dto)?;
        require_scope(&actor, ApiKeyScope::Exec)?;
        let outcome = self
            .service
            .trigger_webhook_owned(&actor, &schedule_id, &dto)
            .await?;
        Ok(Json(outcome))
    }
}
