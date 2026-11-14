//! Thin Ironic HTTP controller for session tasks, fencing leases, steering, and cancellation.

use std::sync::Arc;

use agents_protocol::{ApiKeyScope, CursorPage, FollowUpMessage, SteeringMessage, TaskLease};
use ironic::prelude::*;

use crate::{
    infra::{
        error::{ErrorResponse, validate_dto},
        observability::PerRouteTimeoutInterceptor,
    },
    modules::{
        auth::{
            entities::AuthenticatedActor,
            guards::{AuthGuard, CurrentActor, OptionalIdempotencyKey, ScopeGuard, require_scope},
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
    #[guard(ScopeGuard::new(ApiKeyScope::Write))]
    #[api(
        summary = "Enqueue a task in a session",
        tag = "Tasks",
        security = "bearerAuth"
    )]
    #[body(json = SubmitTaskDto)]
    #[resp(200, "Enqueued task", json = TaskEntity)]
    #[resp(401, "Unauthorized", json = ErrorResponse)]
    #[resp(403, "Forbidden", json = ErrorResponse)]
    #[resp(404, "Session not found", json = ErrorResponse)]
    #[resp(422, "Validation error", json = ErrorResponse)]
    #[resp(500, "Internal server error", json = ErrorResponse)]
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
    #[guard(ScopeGuard::new(ApiKeyScope::Read))]
    #[api(
        summary = "List tasks for a session",
        tag = "Tasks",
        security = "bearerAuth"
    )]
    #[resp(200, "Cursor-paginated tasks", json = CursorPage<TaskEntity>)]
    #[resp(401, "Unauthorized", json = ErrorResponse)]
    #[resp(403, "Forbidden", json = ErrorResponse)]
    #[resp(404, "Session not found", json = ErrorResponse)]
    #[resp(422, "Validation error", json = ErrorResponse)]
    #[resp(500, "Internal server error", json = ErrorResponse)]
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
    #[guard(ScopeGuard::new(ApiKeyScope::Write))]
    #[api(
        summary = "Queue a follow-up prompt for outer loop",
        tag = "Tasks",
        security = "bearerAuth"
    )]
    #[body(json = FollowUpDto)]
    #[resp(200, "Queued follow-up", json = FollowUpMessage)]
    #[resp(401, "Unauthorized", json = ErrorResponse)]
    #[resp(403, "Forbidden", json = ErrorResponse)]
    #[resp(404, "Session not found", json = ErrorResponse)]
    #[resp(422, "Validation error", json = ErrorResponse)]
    #[resp(500, "Internal server error", json = ErrorResponse)]
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
    #[guard(ScopeGuard::new(ApiKeyScope::Read))]
    #[api(summary = "Get task by ID", tag = "Tasks", security = "bearerAuth")]
    #[resp(200, "Task entity", json = TaskEntity)]
    #[resp(401, "Unauthorized", json = ErrorResponse)]
    #[resp(403, "Forbidden", json = ErrorResponse)]
    #[resp(404, "Task not found", json = ErrorResponse)]
    #[resp(500, "Internal server error", json = ErrorResponse)]
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
    #[guard(ScopeGuard::new(ApiKeyScope::Exec))]
    #[api(
        summary = "Acquire a fenced worker lease",
        tag = "Tasks",
        security = "bearerAuth"
    )]
    #[body(json = LeaseTaskDto)]
    #[resp(200, "Granted task lease", json = TaskLease)]
    #[resp(401, "Unauthorized", json = ErrorResponse)]
    #[resp(403, "Forbidden", json = ErrorResponse)]
    #[resp(404, "Task not found", json = ErrorResponse)]
    #[resp(409, "Lease conflict", json = ErrorResponse)]
    #[resp(422, "Validation error", json = ErrorResponse)]
    #[resp(500, "Internal server error", json = ErrorResponse)]
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
    #[guard(ScopeGuard::new(ApiKeyScope::Exec))]
    #[api(
        summary = "Advance fenced task state",
        tag = "Tasks",
        security = "bearerAuth"
    )]
    #[body(json = UpdateTaskStateDto)]
    #[resp(200, "Updated task", json = TaskEntity)]
    #[resp(401, "Unauthorized", json = ErrorResponse)]
    #[resp(403, "Forbidden", json = ErrorResponse)]
    #[resp(404, "Task not found", json = ErrorResponse)]
    #[resp(409, "Fencing conflict", json = ErrorResponse)]
    #[resp(422, "Validation error", json = ErrorResponse)]
    #[resp(500, "Internal server error", json = ErrorResponse)]
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
    #[guard(ScopeGuard::new(ApiKeyScope::Exec))]
    #[interceptor(PerRouteTimeoutInterceptor::secs(120))]
    #[api(
        summary = "Complete fenced task execution",
        tag = "Tasks",
        security = "bearerAuth"
    )]
    #[body(json = CompleteTaskDto)]
    #[resp(200, "Completed task", json = TaskEntity)]
    #[resp(401, "Unauthorized", json = ErrorResponse)]
    #[resp(403, "Forbidden", json = ErrorResponse)]
    #[resp(404, "Task not found", json = ErrorResponse)]
    #[resp(409, "Fencing conflict", json = ErrorResponse)]
    #[resp(422, "Validation error", json = ErrorResponse)]
    #[resp(500, "Internal server error", json = ErrorResponse)]
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
    #[guard(ScopeGuard::new(ApiKeyScope::Write))]
    #[api(
        summary = "Inject live steering message",
        tag = "Tasks",
        security = "bearerAuth"
    )]
    #[body(json = SteerTaskDto)]
    #[resp(200, "Steering message queued", json = SteeringMessage)]
    #[resp(401, "Unauthorized", json = ErrorResponse)]
    #[resp(403, "Forbidden", json = ErrorResponse)]
    #[resp(404, "Task not found", json = ErrorResponse)]
    #[resp(422, "Validation error", json = ErrorResponse)]
    #[resp(500, "Internal server error", json = ErrorResponse)]
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
    #[guard(ScopeGuard::new(ApiKeyScope::Write))]
    #[api(
        summary = "Cancel active or queued task",
        tag = "Tasks",
        security = "bearerAuth"
    )]
    #[resp(200, "Cancelled task", json = TaskEntity)]
    #[resp(401, "Unauthorized", json = ErrorResponse)]
    #[resp(403, "Forbidden", json = ErrorResponse)]
    #[resp(404, "Task not found", json = ErrorResponse)]
    #[resp(500, "Internal server error", json = ErrorResponse)]
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
