//! Thin Ironic HTTP controller for node-based DAG workflows and checkpoint runs.

pub mod ingress;

use std::sync::Arc;

use agents_protocol::ApiKeyScope;
use ironic::prelude::*;

pub use ingress::{SlugRenderController, WebhookIngressController};

use crate::{
    infra::error::validate_dto,
    modules::{
        auth::{
            entities::AuthenticatedActor,
            guards::{AuthGuard, CurrentActor, require_scope},
        },
        workflows::{
            dto::{
                ApproveWorkflowDto, CreateWorkflowDto, PublishRenderSlugDto, TriggerWorkflowDto,
            },
            entities::{RenderedSlugEntity, WorkflowEntity, WorkflowRunEntity},
            services::WorkflowService,
        },
    },
};

#[controller("/api/v1/workflows")]
#[guard(AuthGuard)]
#[derive(Injectable)]
pub struct WorkflowsController {
    service: Arc<WorkflowService>,
}

#[routes]
impl WorkflowsController {
    #[post("/")]
    #[api(
        summary = "Create an automated workflow DAG",
        tag = "Workflows",
        security = "bearerAuth"
    )]
    #[body(json = CreateWorkflowDto)]
    #[resp(200, "Created workflow", json = WorkflowEntity)]
    async fn create(
        &self,
        #[decorator(CurrentActor)] actor: AuthenticatedActor,
        #[body] dto: CreateWorkflowDto,
    ) -> Result<Json<WorkflowEntity>, HttpError> {
        validate_dto(&dto)?;
        require_scope(&actor, ApiKeyScope::Write)?;
        let wf = self.service.create(&actor, &dto).await?;
        Ok(Json(wf))
    }

    #[get("/")]
    #[api(
        summary = "List owned workflows",
        tag = "Workflows",
        security = "bearerAuth"
    )]
    #[resp(200, "Workflow list", json = Vec<WorkflowEntity>)]
    async fn list(
        &self,
        #[decorator(CurrentActor)] actor: AuthenticatedActor,
    ) -> Result<Json<Vec<WorkflowEntity>>, HttpError> {
        require_scope(&actor, ApiKeyScope::Read)?;
        let list = self.service.list_owned(&actor).await?;
        Ok(Json(list))
    }

    #[post("/:workflow_id/approve")]
    #[api(
        summary = "Approve an automated workflow DAG (OWASP ASI-02/ASI-08)",
        tag = "Workflows",
        security = "bearerAuth"
    )]
    #[body(json = ApproveWorkflowDto)]
    #[resp(200, "Approved workflow", json = WorkflowEntity)]
    async fn approve(
        &self,
        #[decorator(CurrentActor)] actor: AuthenticatedActor,
        #[param] workflow_id: String,
        #[body] dto: Option<ApproveWorkflowDto>,
    ) -> Result<Json<WorkflowEntity>, HttpError> {
        let _ = dto;
        require_scope(&actor, ApiKeyScope::Write)?;
        let wf = self.service.approve(&actor, &workflow_id).await?;
        Ok(Json(wf))
    }

    #[post("/:workflow_id/render")]
    #[api(
        summary = "Publish rendered HTML content to workflow render slug",
        tag = "Workflows",
        security = "bearerAuth"
    )]
    #[body(json = PublishRenderSlugDto)]
    #[resp(200, "Rendered slug entity", json = RenderedSlugEntity)]
    async fn publish_render(
        &self,
        #[decorator(CurrentActor)] actor: AuthenticatedActor,
        #[param] workflow_id: String,
        #[body] dto: PublishRenderSlugDto,
    ) -> Result<Json<RenderedSlugEntity>, HttpError> {
        validate_dto(&dto)?;
        require_scope(&actor, ApiKeyScope::Write)?;
        let res = self
            .service
            .publish_render(&actor, &workflow_id, &dto)
            .await?;
        Ok(Json(res))
    }

    #[post("/:workflow_id/trigger")]
    #[api(
        summary = "Execute workflow DAG manually",
        tag = "Workflows",
        security = "bearerAuth"
    )]
    #[body(json = TriggerWorkflowDto)]
    #[resp(200, "Workflow run summary", json = WorkflowRunEntity)]
    async fn trigger(
        &self,
        #[decorator(CurrentActor)] actor: AuthenticatedActor,
        #[param] workflow_id: String,
        #[body] dto: TriggerWorkflowDto,
    ) -> Result<Json<WorkflowRunEntity>, HttpError> {
        validate_dto(&dto)?;
        require_scope(&actor, ApiKeyScope::Exec)?;
        let run = self
            .service
            .execute_owned(
                &actor,
                &workflow_id,
                "manual-api",
                None,
                dto.input.unwrap_or_default(),
            )
            .await?;
        Ok(Json(run))
    }

    #[get("/runs/:run_id")]
    #[api(
        summary = "Inspect workflow run checkpoint",
        tag = "Workflows",
        security = "bearerAuth"
    )]
    #[resp(200, "Workflow run state", json = WorkflowRunEntity)]
    async fn get_run(
        &self,
        #[decorator(CurrentActor)] actor: AuthenticatedActor,
        #[param] run_id: String,
    ) -> Result<Json<WorkflowRunEntity>, HttpError> {
        require_scope(&actor, ApiKeyScope::Read)?;
        let run = self.service.get_run_owned(&actor, &run_id).await?;
        Ok(Json(run))
    }
}
