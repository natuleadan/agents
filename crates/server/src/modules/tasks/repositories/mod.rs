//! Task persistence repository supporting fencing token checks, idempotency, and pagination.

pub mod memory;
pub mod pg;

use std::sync::Arc;

use agents_protocol::{CursorPage, FencingToken, InteractionMode, StopReason, TaskState};

use crate::{
    infra::{error::ServerError, state::BoxFuture},
    modules::tasks::entities::{TaskEntity, is_valid_transition},
};

pub use memory::InMemoryTaskRepository;
pub use pg::PgTaskRepository;

/// Concrete DI wrapper around [`TaskRepository`].
#[derive(Clone)]
pub struct TaskStore(pub Arc<dyn TaskRepository>);

impl Default for TaskStore {
    fn default() -> Self {
        Self(InMemoryTaskRepository::new())
    }
}

/// Persistence contract for durable fenced tasks.
pub trait TaskRepository: Send + Sync + 'static {
    fn create<'a>(&'a self, task: TaskEntity) -> BoxFuture<'a, Result<TaskEntity, ServerError>>;
    fn find_by_id<'a>(
        &'a self,
        id: &'a str,
    ) -> BoxFuture<'a, Result<Option<TaskEntity>, ServerError>>;
    fn find_by_idempotency<'a>(
        &'a self,
        session_id: &'a str,
        idempotency_key: &'a str,
    ) -> BoxFuture<'a, Result<Option<TaskEntity>, ServerError>>;
    fn list_for_session<'a>(
        &'a self,
        session_id: &'a str,
        cursor: Option<&'a str>,
        limit: u32,
    ) -> BoxFuture<'a, Result<CursorPage<TaskEntity>, ServerError>>;
    fn acquire_lease<'a>(
        &'a self,
        task_id: &'a str,
        boxer_node_id: &'a str,
        now_ms: u64,
        lease_expires_at: u64,
    ) -> BoxFuture<'a, Result<TaskEntity, ServerError>>;
    fn transition_fenced<'a>(
        &'a self,
        task_id: &'a str,
        fencing_token: FencingToken,
        next_state: TaskState,
        now_ms: u64,
    ) -> BoxFuture<'a, Result<TaskEntity, ServerError>>;
    fn complete_fenced<'a>(
        &'a self,
        task_id: &'a str,
        fencing_token: FencingToken,
        exit_code: i32,
        stop_reason: StopReason,
        output_summary: Option<String>,
        now_ms: u64,
    ) -> BoxFuture<'a, Result<TaskEntity, ServerError>>;
    fn cancel<'a>(
        &'a self,
        task_id: &'a str,
        now_ms: u64,
    ) -> BoxFuture<'a, Result<TaskEntity, ServerError>>;
}

pub(crate) fn validate_fenced_mutation(
    current: &TaskEntity,
    provided_token: FencingToken,
    next_state: &TaskState,
    now_ms: u64,
) -> Result<(), ServerError> {
    if current.is_terminal() {
        return Err(ServerError::Conflict(format!(
            "task `{}` is already terminal (`{}`)",
            current.id,
            current.state.as_str()
        )));
    }
    if current.fencing_token != provided_token {
        return Err(ServerError::Conflict(format!(
            "stale fencing token {}: active token is {}",
            provided_token.0, current.fencing_token.0
        )));
    }
    if current.lease_expires_at.is_some_and(|exp| exp <= now_ms) {
        return Err(ServerError::Conflict(
            "task lease expired before state update".into(),
        ));
    }
    if !is_valid_transition(&current.state, next_state) {
        return Err(ServerError::Conflict(format!(
            "invalid transition `{}` -> `{}`",
            current.state.as_str(),
            next_state.as_str()
        )));
    }
    Ok(())
}

pub(crate) const fn mode_to_str(mode: InteractionMode) -> &'static str {
    match mode {
        InteractionMode::Plan => "plan",
        InteractionMode::Act => "act",
    }
}

pub(crate) fn encode_cursor(task: &TaskEntity) -> String {
    format!("{}:{}", task.created_at, task.id)
}

pub(crate) fn parse_cursor(cursor: Option<&str>) -> (Option<u64>, Option<&str>) {
    let Some(raw) = cursor else {
        return (None, None);
    };
    let Some((ts_str, id)) = raw.split_once(':') else {
        return (None, None);
    };
    match ts_str.parse::<u64>() {
        Ok(ts) => (Some(ts), Some(id)),
        Err(_) => (None, None),
    }
}
