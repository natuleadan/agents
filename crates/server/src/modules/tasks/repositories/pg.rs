//! PostgreSQL implementation of [`TaskRepository`].

use std::sync::Arc;

use agents_protocol::{CursorPage, FencingToken, InteractionMode, StopReason, TaskState};
use sqlx::{PgPool, Row};

use crate::{
    infra::{error::ServerError, state::BoxFuture},
    modules::tasks::{
        entities::TaskEntity,
        repositories::{
            TaskRepository, encode_cursor, mode_to_str, parse_cursor, validate_fenced_mutation,
        },
    },
};

/// PostgreSQL task repository enforcing fencing tokens and idempotency uniqueness.
#[derive(Clone)]
pub struct PgTaskRepository {
    pool: PgPool,
}

impl PgTaskRepository {
    #[must_use]
    pub fn new(pool: PgPool) -> Arc<Self> {
        Arc::new(Self { pool })
    }
}

impl TaskRepository for PgTaskRepository {
    fn create<'a>(&'a self, task: TaskEntity) -> BoxFuture<'a, Result<TaskEntity, ServerError>> {
        Box::pin(async move {
            sqlx::query(
                "INSERT INTO tasks (id, session_id, owner_id, prompt, state, mode, fencing_token, lease_holder, lease_expires_at, idempotency_key, version, exit_code, stop_reason, output_summary, created_at, updated_at)
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16)",
            )
            .bind(&task.id)
            .bind(&task.session_id)
            .bind(&task.owner_id)
            .bind(&task.prompt)
            .bind(task.state.as_str())
            .bind(mode_to_str(task.mode))
            .bind(task.fencing_token.0 as i64)
            .bind(&task.lease_holder)
            .bind(task.lease_expires_at.map(|v| v as i64))
            .bind(&task.idempotency_key)
            .bind(task.version as i64)
            .bind(task.exit_code)
            .bind(task.stop_reason.as_ref().map(StopReason::as_str))
            .bind(&task.output_summary)
            .bind(task.created_at as i64)
            .bind(task.updated_at as i64)
            .execute(&self.pool)
            .await
            .map_err(|e| {
                let msg = e.to_string();
                if msg.contains("uq_tasks_session_idempotency") {
                    ServerError::Conflict("duplicate idempotency key for session".into())
                } else {
                    ServerError::Internal(format!("create task failed: {msg}"))
                }
            })?;
            Ok(task)
        })
    }

    fn find_by_id<'a>(
        &'a self,
        id: &'a str,
    ) -> BoxFuture<'a, Result<Option<TaskEntity>, ServerError>> {
        Box::pin(async move {
            let row = sqlx::query(
                "SELECT id, session_id, owner_id, prompt, state, mode, fencing_token, lease_holder, lease_expires_at, idempotency_key, version, exit_code, stop_reason, output_summary, created_at, updated_at
                 FROM tasks WHERE id = $1",
            )
            .bind(id)
            .fetch_optional(&self.pool)
            .await
            .map_err(|e| ServerError::Internal(format!("find task failed: {e}")))?;

            row.map(row_to_task).transpose()
        })
    }

    fn find_by_idempotency<'a>(
        &'a self,
        session_id: &'a str,
        idempotency_key: &'a str,
    ) -> BoxFuture<'a, Result<Option<TaskEntity>, ServerError>> {
        Box::pin(async move {
            let row = sqlx::query(
                "SELECT id, session_id, owner_id, prompt, state, mode, fencing_token, lease_holder, lease_expires_at, idempotency_key, version, exit_code, stop_reason, output_summary, created_at, updated_at
                 FROM tasks WHERE session_id = $1 AND idempotency_key = $2",
            )
            .bind(session_id)
            .bind(idempotency_key)
            .fetch_optional(&self.pool)
            .await
            .map_err(|e| ServerError::Internal(format!("find task idempotency failed: {e}")))?;

            row.map(row_to_task).transpose()
        })
    }

    fn list_for_session<'a>(
        &'a self,
        session_id: &'a str,
        cursor: Option<&'a str>,
        limit: u32,
    ) -> BoxFuture<'a, Result<CursorPage<TaskEntity>, ServerError>> {
        Box::pin(async move {
            let page_size = i64::from(limit.clamp(1, 100));
            let (cursor_ts, cursor_id) = parse_cursor(cursor);
            let rows = sqlx::query(
                "SELECT id, session_id, owner_id, prompt, state, mode, fencing_token, lease_holder, lease_expires_at, idempotency_key, version, exit_code, stop_reason, output_summary, created_at, updated_at
                 FROM tasks
                 WHERE session_id = $1
                   AND ($2::bigint IS NULL OR (created_at, id) < ($2, $3))
                 ORDER BY created_at DESC, id DESC
                 LIMIT $4",
            )
            .bind(session_id)
            .bind(cursor_ts.map(|v| v as i64))
            .bind(cursor_id.unwrap_or_default())
            .bind(page_size + 1)
            .fetch_all(&self.pool)
            .await
            .map_err(|e| ServerError::Internal(format!("list tasks failed: {e}")))?;

            let mut items = rows
                .into_iter()
                .map(row_to_task)
                .collect::<Result<Vec<_>, _>>()?;
            let next_cursor = if items.len() > page_size as usize {
                items.pop();
                items.last().map(encode_cursor)
            } else {
                None
            };
            Ok(CursorPage {
                items,
                next_cursor,
                backwards_cursor: cursor.map(ToOwned::to_owned),
                total_hint: None,
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
            let row = sqlx::query(
                "UPDATE tasks
                 SET state = 'leased',
                     fencing_token = fencing_token + 1,
                     lease_holder = $2,
                     lease_expires_at = $3,
                     version = version + 1,
                     updated_at = $4
                 WHERE id = $1
                   AND (state = 'queued' OR (state IN ('leased', 'running', 'blocked') AND lease_expires_at IS NOT NULL AND lease_expires_at <= $4))
                 RETURNING id, session_id, owner_id, prompt, state, mode, fencing_token, lease_holder, lease_expires_at, idempotency_key, version, exit_code, stop_reason, output_summary, created_at, updated_at",
            )
            .bind(task_id)
            .bind(boxer_node_id)
            .bind(lease_expires_at as i64)
            .bind(now_ms as i64)
            .fetch_optional(&self.pool)
            .await
            .map_err(|e| ServerError::Internal(format!("acquire lease failed: {e}")))?;

            match row {
                Some(r) => row_to_task(r),
                None => Err(ServerError::Conflict(format!(
                    "task `{task_id}` cannot be leased (already leased or terminal)"
                ))),
            }
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
            let current = self
                .find_by_id(task_id)
                .await?
                .ok_or_else(|| ServerError::NotFound(format!("task `{task_id}` not found")))?;

            validate_fenced_mutation(&current, fencing_token, &next_state, now_ms)?;
            let row = sqlx::query(
                "UPDATE tasks
                 SET state = $3, version = version + 1, updated_at = $4
                 WHERE id = $1 AND fencing_token = $2 AND version = $5
                 RETURNING id, session_id, owner_id, prompt, state, mode, fencing_token, lease_holder, lease_expires_at, idempotency_key, version, exit_code, stop_reason, output_summary, created_at, updated_at",
            )
            .bind(task_id)
            .bind(fencing_token.0 as i64)
            .bind(next_state.as_str())
            .bind(now_ms as i64)
            .bind(current.version as i64)
            .fetch_optional(&self.pool)
            .await
            .map_err(|e| ServerError::Internal(format!("transition_fenced failed: {e}")))?;

            match row {
                Some(r) => row_to_task(r),
                None => Err(ServerError::Conflict(
                    "fencing token mismatch or concurrent modification".into(),
                )),
            }
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
            let current = self
                .find_by_id(task_id)
                .await?
                .ok_or_else(|| ServerError::NotFound(format!("task `{task_id}` not found")))?;
            let next_state = if exit_code == 0 && stop_reason != StopReason::Error {
                TaskState::Done
            } else {
                TaskState::Failed
            };
            validate_fenced_mutation(&current, fencing_token, &next_state, now_ms)?;

            let row = sqlx::query(
                "UPDATE tasks
                 SET state = $3, exit_code = $4, stop_reason = $5, output_summary = $6, version = version + 1, updated_at = $7
                 WHERE id = $1 AND fencing_token = $2 AND version = $8
                 RETURNING id, session_id, owner_id, prompt, state, mode, fencing_token, lease_holder, lease_expires_at, idempotency_key, version, exit_code, stop_reason, output_summary, created_at, updated_at",
            )
            .bind(task_id)
            .bind(fencing_token.0 as i64)
            .bind(next_state.as_str())
            .bind(exit_code)
            .bind(stop_reason.as_str())
            .bind(output_summary)
            .bind(now_ms as i64)
            .bind(current.version as i64)
            .fetch_optional(&self.pool)
            .await
            .map_err(|e| ServerError::Internal(format!("complete_fenced failed: {e}")))?;

            match row {
                Some(r) => row_to_task(r),
                None => Err(ServerError::Conflict(
                    "fencing token mismatch or concurrent completion".into(),
                )),
            }
        })
    }

    fn cancel<'a>(
        &'a self,
        task_id: &'a str,
        now_ms: u64,
    ) -> BoxFuture<'a, Result<TaskEntity, ServerError>> {
        Box::pin(async move {
            let current = self
                .find_by_id(task_id)
                .await?
                .ok_or_else(|| ServerError::NotFound(format!("task `{task_id}` not found")))?;
            if current.is_terminal() {
                return Err(ServerError::Conflict(format!(
                    "task `{task_id}` already in terminal state `{}`",
                    current.state.as_str()
                )));
            }
            let row = sqlx::query(
                "UPDATE tasks
                 SET state = 'cancelled', stop_reason = 'aborted', version = version + 1, updated_at = $2
                 WHERE id = $1 AND state NOT IN ('done', 'failed', 'cancelled')
                 RETURNING id, session_id, owner_id, prompt, state, mode, fencing_token, lease_holder, lease_expires_at, idempotency_key, version, exit_code, stop_reason, output_summary, created_at, updated_at",
            )
            .bind(task_id)
            .bind(now_ms as i64)
            .fetch_optional(&self.pool)
            .await
            .map_err(|e| ServerError::Internal(format!("cancel task failed: {e}")))?;

            match row {
                Some(r) => row_to_task(r),
                None => Err(ServerError::Conflict("task concurrently terminated".into())),
            }
        })
    }
}

fn row_to_task(row: sqlx::postgres::PgRow) -> Result<TaskEntity, ServerError> {
    let state_str: String = row
        .try_get("state")
        .map_err(|e| ServerError::Internal(e.to_string()))?;
    let mode_str: String = row
        .try_get("mode")
        .map_err(|e| ServerError::Internal(e.to_string()))?;
    let fencing: i64 = row
        .try_get("fencing_token")
        .map_err(|e| ServerError::Internal(e.to_string()))?;
    let lease_exp: Option<i64> = row
        .try_get("lease_expires_at")
        .map_err(|e| ServerError::Internal(e.to_string()))?;
    let version: i64 = row
        .try_get("version")
        .map_err(|e| ServerError::Internal(e.to_string()))?;
    let stop_str: Option<String> = row
        .try_get("stop_reason")
        .map_err(|e| ServerError::Internal(e.to_string()))?;
    let created: i64 = row
        .try_get("created_at")
        .map_err(|e| ServerError::Internal(e.to_string()))?;
    let updated: i64 = row
        .try_get("updated_at")
        .map_err(|e| ServerError::Internal(e.to_string()))?;

    Ok(TaskEntity {
        id: row
            .try_get("id")
            .map_err(|e| ServerError::Internal(e.to_string()))?,
        session_id: row
            .try_get("session_id")
            .map_err(|e| ServerError::Internal(e.to_string()))?,
        owner_id: row
            .try_get("owner_id")
            .map_err(|e| ServerError::Internal(e.to_string()))?,
        prompt: row
            .try_get("prompt")
            .map_err(|e| ServerError::Internal(e.to_string()))?,
        state: parse_task_state(&state_str),
        mode: if mode_str == "plan" {
            InteractionMode::Plan
        } else {
            InteractionMode::Act
        },
        fencing_token: FencingToken(fencing.max(0) as u64),
        lease_holder: row
            .try_get("lease_holder")
            .map_err(|e| ServerError::Internal(e.to_string()))?,
        lease_expires_at: lease_exp.map(|v| v.max(0) as u64),
        idempotency_key: row
            .try_get("idempotency_key")
            .map_err(|e| ServerError::Internal(e.to_string()))?,
        version: version.max(1) as u64,
        exit_code: row
            .try_get("exit_code")
            .map_err(|e| ServerError::Internal(e.to_string()))?,
        stop_reason: stop_str.map(|s| parse_stop_reason(&s)),
        output_summary: row
            .try_get("output_summary")
            .map_err(|e| ServerError::Internal(e.to_string()))?,
        created_at: created.max(0) as u64,
        updated_at: updated.max(0) as u64,
    })
}

fn parse_task_state(raw: &str) -> TaskState {
    match raw {
        "queued" => TaskState::Queued,
        "leased" => TaskState::Leased,
        "running" => TaskState::Running,
        "blocked" => TaskState::Blocked,
        "done" => TaskState::Done,
        "failed" => TaskState::Failed,
        "cancelled" => TaskState::Cancelled,
        other => TaskState::Unknown(other.to_owned()),
    }
}

fn parse_stop_reason(raw: &str) -> StopReason {
    match raw {
        "end_turn" => StopReason::EndTurn,
        "tool_use" => StopReason::ToolUse,
        "length" => StopReason::Length,
        "aborted" => StopReason::Aborted,
        "error" => StopReason::Error,
        other => StopReason::Unknown(other.to_owned()),
    }
}
