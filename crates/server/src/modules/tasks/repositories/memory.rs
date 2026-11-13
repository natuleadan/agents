//! In-memory implementation of [`TaskRepository`] for deterministic unit/integration tests.

use std::sync::Arc;

use agents_protocol::{CursorPage, FencingToken, StopReason, TaskState};
use tokio::sync::Mutex;

use crate::{
    infra::{error::ServerError, state::BoxFuture},
    modules::tasks::{
        entities::TaskEntity,
        repositories::{TaskRepository, encode_cursor, parse_cursor, validate_fenced_mutation},
    },
};

/// Deterministic in-memory task repository used in tests and standalone runs.
#[derive(Default)]
pub struct InMemoryTaskRepository {
    tasks: Mutex<Vec<TaskEntity>>,
}

impl InMemoryTaskRepository {
    #[must_use]
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }
}

impl TaskRepository for InMemoryTaskRepository {
    fn create<'a>(&'a self, task: TaskEntity) -> BoxFuture<'a, Result<TaskEntity, ServerError>> {
        Box::pin(async move {
            let mut guard = self.tasks.lock().await;
            if let Some(ref idem) = task.idempotency_key
                && guard.iter().any(|t| {
                    t.session_id == task.session_id && t.idempotency_key.as_deref() == Some(idem)
                })
            {
                return Err(ServerError::Conflict(
                    "duplicate idempotency key for session".into(),
                ));
            }
            guard.push(task.clone());
            Ok(task)
        })
    }

    fn find_by_id<'a>(
        &'a self,
        id: &'a str,
    ) -> BoxFuture<'a, Result<Option<TaskEntity>, ServerError>> {
        Box::pin(async move { Ok(self.tasks.lock().await.iter().find(|t| t.id == id).cloned()) })
    }

    fn find_by_idempotency<'a>(
        &'a self,
        session_id: &'a str,
        idempotency_key: &'a str,
    ) -> BoxFuture<'a, Result<Option<TaskEntity>, ServerError>> {
        Box::pin(async move {
            Ok(self
                .tasks
                .lock()
                .await
                .iter()
                .find(|t| {
                    t.session_id == session_id
                        && t.idempotency_key.as_deref() == Some(idempotency_key)
                })
                .cloned())
        })
    }

    fn list_for_session<'a>(
        &'a self,
        session_id: &'a str,
        cursor: Option<&'a str>,
        limit: u32,
    ) -> BoxFuture<'a, Result<CursorPage<TaskEntity>, ServerError>> {
        Box::pin(async move {
            let guard = self.tasks.lock().await;
            let (cursor_ts, cursor_id) = parse_cursor(cursor);
            let mut items: Vec<TaskEntity> = guard
                .iter()
                .filter(|t| t.session_id == session_id)
                .filter(|t| match (cursor_ts, cursor_id) {
                    (Some(ts), Some(id)) => (t.created_at, t.id.as_str()) < (ts, id),
                    _ => true,
                })
                .cloned()
                .collect();
            items.sort_by(|a, b| (b.created_at, &b.id).cmp(&(a.created_at, &a.id)));
            let total_hint = Some(items.len() as u64);
            let max = limit.clamp(1, 100) as usize;
            let has_more = items.len() > max;
            items.truncate(max);
            let next_cursor = if has_more {
                items.last().map(encode_cursor)
            } else {
                None
            };
            Ok(CursorPage {
                items,
                next_cursor,
                backwards_cursor: cursor.map(ToOwned::to_owned),
                total_hint,
            })
        })
    }

    fn acquire_lease<'a>(
        &'a self,
        task_id: &'a str,
        boxer_node_id: &'a str,
        now_ms: u64,
        lease_expires_at: u64,
    ) -> BoxFuture<'a, Result<TaskEntity, ServerError>> {
        Box::pin(async move {
            let mut guard = self.tasks.lock().await;
            let Some(task) = guard.iter_mut().find(|t| t.id == task_id) else {
                return Err(ServerError::NotFound(format!("task `{task_id}` not found")));
            };
            let lease_expired = matches!(
                task.state,
                TaskState::Leased | TaskState::Running | TaskState::Blocked
            ) && task.lease_expires_at.is_some_and(|exp| exp <= now_ms);
            if task.state != TaskState::Queued && !lease_expired {
                return Err(ServerError::Conflict(format!(
                    "task `{task_id}` cannot be leased from state `{}`",
                    task.state.as_str()
                )));
            }
            task.state = TaskState::Leased;
            task.fencing_token = FencingToken(task.fencing_token.0.saturating_add(1));
            task.lease_holder = Some(boxer_node_id.to_owned());
            task.lease_expires_at = Some(lease_expires_at);
            task.version = task.version.saturating_add(1);
            task.updated_at = now_ms;
            Ok(task.clone())
        })
    }

    fn transition_fenced<'a>(
        &'a self,
        task_id: &'a str,
        fencing_token: FencingToken,
        next_state: TaskState,
        now_ms: u64,
    ) -> BoxFuture<'a, Result<TaskEntity, ServerError>> {
        Box::pin(async move {
            let mut guard = self.tasks.lock().await;
            let Some(task) = guard.iter_mut().find(|t| t.id == task_id) else {
                return Err(ServerError::NotFound(format!("task `{task_id}` not found")));
            };
            validate_fenced_mutation(task, fencing_token, &next_state, now_ms)?;
            task.state = next_state;
            task.version = task.version.saturating_add(1);
            task.updated_at = now_ms;
            Ok(task.clone())
        })
    }

    fn complete_fenced<'a>(
        &'a self,
        task_id: &'a str,
        fencing_token: FencingToken,
        exit_code: i32,
        stop_reason: StopReason,
        output_summary: Option<String>,
        now_ms: u64,
    ) -> BoxFuture<'a, Result<TaskEntity, ServerError>> {
        Box::pin(async move {
            let mut guard = self.tasks.lock().await;
            let Some(task) = guard.iter_mut().find(|t| t.id == task_id) else {
                return Err(ServerError::NotFound(format!("task `{task_id}` not found")));
            };
            let next_state = if exit_code == 0 && stop_reason != StopReason::Error {
                TaskState::Done
            } else {
                TaskState::Failed
            };
            validate_fenced_mutation(task, fencing_token, &next_state, now_ms)?;
            task.state = next_state;
            task.exit_code = Some(exit_code);
            task.stop_reason = Some(stop_reason);
            task.output_summary = output_summary;
            task.version = task.version.saturating_add(1);
            task.updated_at = now_ms;
            Ok(task.clone())
        })
    }

    fn cancel<'a>(
        &'a self,
        task_id: &'a str,
        now_ms: u64,
    ) -> BoxFuture<'a, Result<TaskEntity, ServerError>> {
        Box::pin(async move {
            let mut guard = self.tasks.lock().await;
            let Some(task) = guard.iter_mut().find(|t| t.id == task_id) else {
                return Err(ServerError::NotFound(format!("task `{task_id}` not found")));
            };
            if task.is_terminal() {
                return Err(ServerError::Conflict(format!(
                    "task `{task_id}` is already terminal"
                )));
            }
            task.state = TaskState::Cancelled;
            task.stop_reason = Some(StopReason::Aborted);
            task.version = task.version.saturating_add(1);
            task.updated_at = now_ms;
            Ok(task.clone())
        })
    }
}
