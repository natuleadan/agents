//! Thin Ironic HTTP controller for session tasks, fencing leases, steering, and cancellation.

use std::sync::Arc;

use agents_protocol::{ApiKeyScope, CursorPage, FollowUpMessage, SteeringMessage, TaskLease};
use ironic::prelude::*;

use crate::{
    infra::error::validate_dto,
    modules::{
        auth::{
            entities::AuthenticatedActor,
            guards::{AuthGuard, CurrentActor, OptionalIdempotencyKey, require_scope},
        },
        tasks::{
            dto::{
                CompleteTaskDto, FollowUpDto, LeaseTaskDto, ListTasksQuery, SteerTaskDto,
                SubmitTaskDto, UpdateTaskStateDto,
            },
            entities::TaskEntity,
            services::TaskService,
        },
    },
};

#[controller("/api/v1")]
#[guard(AuthGuard)]
#[derive(Injectable)]
pub struct TasksController {
    service: Arc<TaskService>,
}

#[routes]
impl TasksController {
    #[post("/sessions/:session_id/tasks")]
    #[api(
        summary = "Enqueue a task in a session",
        tag = "Tasks",
        security = "bearerAuth"
    )]
    #[body(json = SubmitTaskDto)]
    #[resp(200, "Enqueued task", json = TaskEntity)]
    async fn submit(
        &self,
        #[decorator(CurrentActor)] actor: AuthenticatedActor,
        #[decorator(OptionalIdempotencyKey)] idempotency_key: Option<String>,
        #[param] session_id: String,
        #[body] dto: SubmitTaskDto,
    ) -> Result<Json<TaskEntity>, HttpError> {
        validate_dto(&dto)?;
        require_scope(&actor, ApiKeyScope::Write)?;
        let task = self
            .service
            .submit(&actor, &session_id, idempotency_key.as_deref(), &dto)
            .await?;
        Ok(Json(task))
    }

    #[get("/sessions/:session_id/tasks")]
    #[api(
        summary = "List tasks for a session",
        tag = "Tasks",
        security = "bearerAuth"
    )]
    #[resp(200, "Cursor-paginated tasks", json = CursorPage<TaskEntity>)]
    async fn list_for_session(
        &self,
        #[decorator(CurrentActor)] actor: AuthenticatedActor,
        #[param] session_id: String,
        #[query] query: ListTasksQuery,
    ) -> Result<Json<CursorPage<TaskEntity>>, HttpError> {
        validate_dto(&query)?;
        require_scope(&actor, ApiKeyScope::Read)?;
        let page = self
            .service
            .list_for_session(&actor, &session_id, &query)
            .await?;
        Ok(Json(page))
    }

    #[post("/sessions/:session_id/follow-ups")]
    #[api(
        summary = "Queue a follow-up prompt for outer loop",
        tag = "Tasks",
        security = "bearerAuth"
    )]
    #[body(json = FollowUpDto)]
    #[resp(200, "Queued follow-up", json = FollowUpMessage)]
    async fn follow_up(
        &self,
        #[decorator(CurrentActor)] actor: AuthenticatedActor,
        #[param] session_id: String,
        #[body] dto: FollowUpDto,
    ) -> Result<Json<FollowUpMessage>, HttpError> {
        validate_dto(&dto)?;
        require_scope(&actor, ApiKeyScope::Write)?;
        let msg = self
            .service
            .queue_follow_up(&actor, &session_id, &dto)
            .await?;
        Ok(Json(msg))
    }

    #[get("/tasks/:id")]
    #[api(summary = "Get task by ID", tag = "Tasks", security = "bearerAuth")]
    #[resp(200, "Task entity", json = TaskEntity)]
    async fn get(
        &self,
        #[decorator(CurrentActor)] actor: AuthenticatedActor,
        #[param] id: String,
    ) -> Result<Json<TaskEntity>, HttpError> {
        require_scope(&actor, ApiKeyScope::Read)?;
        let task = self.service.get_owned(&actor, &id).await?;
        Ok(Json(task))
    }

    #[post("/tasks/:id/lease")]
    #[api(
        summary = "Acquire a fenced worker lease",
        tag = "Tasks",
        security = "bearerAuth"
    )]
    #[body(json = LeaseTaskDto)]
    #[resp(200, "Granted task lease", json = TaskLease)]
    #[resp(409, "Lease conflict")]
    async fn lease(
        &self,
        #[decorator(CurrentActor)] actor: AuthenticatedActor,
        #[param] id: String,
        #[body] dto: LeaseTaskDto,
    ) -> Result<Json<TaskLease>, HttpError> {
        validate_dto(&dto)?;
        require_scope(&actor, ApiKeyScope::Exec)?;
        let lease = self.service.lease(&id, &dto).await?;
        Ok(Json(lease))
    }

    #[post("/tasks/:id/state")]
    #[api(
        summary = "Advance fenced task state",
        tag = "Tasks",
        security = "bearerAuth"
    )]
    #[body(json = UpdateTaskStateDto)]
    #[resp(200, "Updated task", json = TaskEntity)]
    #[resp(409, "Fencing conflict")]
    async fn transition(
        &self,
        #[decorator(CurrentActor)] actor: AuthenticatedActor,
        #[param] id: String,
        #[body] dto: UpdateTaskStateDto,
    ) -> Result<Json<TaskEntity>, HttpError> {
        validate_dto(&dto)?;
        require_scope(&actor, ApiKeyScope::Exec)?;
        let task = self.service.transition_fenced(&id, &dto).await?;
        Ok(Json(task))
    }

    #[post("/tasks/:id/complete")]
    #[api(
        summary = "Complete fenced task execution",
        tag = "Tasks",
        security = "bearerAuth"
    )]
    #[body(json = CompleteTaskDto)]
    #[resp(200, "Completed task", json = TaskEntity)]
    #[resp(409, "Fencing conflict")]
    async fn complete(
        &self,
        #[decorator(CurrentActor)] actor: AuthenticatedActor,
        #[param] id: String,
        #[body] dto: CompleteTaskDto,
    ) -> Result<Json<TaskEntity>, HttpError> {
        validate_dto(&dto)?;
        require_scope(&actor, ApiKeyScope::Exec)?;
        let task = self.service.complete_fenced(&id, &dto).await?;
        Ok(Json(task))
    }

    #[post("/tasks/:id/steer")]
    #[api(
        summary = "Inject live steering message",
        tag = "Tasks",
        security = "bearerAuth"
    )]
    #[body(json = SteerTaskDto)]
    #[resp(200, "Steering message queued", json = SteeringMessage)]
    async fn steer(
        &self,
        #[decorator(CurrentActor)] actor: AuthenticatedActor,
        #[param] id: String,
        #[body] dto: SteerTaskDto,
    ) -> Result<Json<SteeringMessage>, HttpError> {
        validate_dto(&dto)?;
        require_scope(&actor, ApiKeyScope::Write)?;
        let msg = self.service.steer(&actor, &id, &dto).await?;
        Ok(Json(msg))
    }

    #[post("/tasks/:id/cancel")]
    #[api(
        summary = "Cancel active or queued task",
        tag = "Tasks",
        security = "bearerAuth"
    )]
    #[resp(200, "Cancelled task", json = TaskEntity)]
    async fn cancel(
        &self,
        #[decorator(CurrentActor)] actor: AuthenticatedActor,
        #[param] id: String,
    ) -> Result<Json<TaskEntity>, HttpError> {
        require_scope(&actor, ApiKeyScope::Write)?;
        let task = self.service.cancel(&actor, &id).await?;
        Ok(Json(task))
    }
}
