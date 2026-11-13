//! Persistence repository for sessions (PostgreSQL + in-memory test store).

use std::sync::Arc;

use agents_protocol::{CursorPage, InteractionMode, TriStateFilter};
use sqlx::{PgPool, Row};
use tokio::sync::Mutex;

use crate::{
    infra::{error::ServerError, state::BoxFuture},
    modules::sessions::entities::SessionEntity,
};

/// Concrete DI wrapper around [`SessionRepository`].
#[derive(Clone)]
pub struct SessionStore(pub Arc<dyn SessionRepository>);

impl Default for SessionStore {
    fn default() -> Self {
        Self(InMemorySessionRepository::new())
    }
}

/// Persistence operations for sessions and durable draft snapshots.
pub trait SessionRepository: Send + Sync + 'static {
    fn create<'a>(
        &'a self,
        session: SessionEntity,
    ) -> BoxFuture<'a, Result<SessionEntity, ServerError>>;
    fn find_by_id<'a>(
        &'a self,
        id: &'a str,
    ) -> BoxFuture<'a, Result<Option<SessionEntity>, ServerError>>;
    fn list_for_owner<'a>(
        &'a self,
        owner_id: &'a str,
        archived: TriStateFilter<bool>,
        cursor: Option<&'a str>,
        limit: u32,
    ) -> BoxFuture<'a, Result<CursorPage<SessionEntity>, ServerError>>;
    fn update<'a>(
        &'a self,
        session: SessionEntity,
        expected_version: Option<u64>,
    ) -> BoxFuture<'a, Result<SessionEntity, ServerError>>;
    fn save_draft<'a>(
        &'a self,
        id: &'a str,
        content: &'a str,
        updated_at: u64,
    ) -> BoxFuture<'a, Result<(), ServerError>>;
    fn delete<'a>(&'a self, id: &'a str) -> BoxFuture<'a, Result<bool, ServerError>>;
}

/// PostgreSQL session repository supporting cursor pagination and optimistic concurrency.
#[derive(Clone)]
pub struct PgSessionRepository {
    pool: PgPool,
}

impl PgSessionRepository {
    #[must_use]
    pub fn new(pool: PgPool) -> Arc<Self> {
        Arc::new(Self { pool })
    }
}

impl SessionRepository for PgSessionRepository {
    fn create<'a>(
        &'a self,
        session: SessionEntity,
    ) -> BoxFuture<'a, Result<SessionEntity, ServerError>> {
        Box::pin(async move {
            sqlx::query(
                "INSERT INTO sessions (id, owner_id, title, mode, archived_at, draft_content, draft_updated_at, version, created_at, updated_at)
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)",
            )
            .bind(&session.id)
            .bind(&session.owner_id)
            .bind(&session.title)
            .bind(mode_to_str(session.mode))
            .bind(session.archived_at.map(|v| v as i64))
            .bind(&session.draft_content)
            .bind(session.draft_updated_at.map(|v| v as i64))
            .bind(session.version as i64)
            .bind(session.created_at as i64)
            .bind(session.updated_at as i64)
            .execute(&self.pool)
            .await
            .map_err(|e| ServerError::Internal(format!("create session failed: {e}")))?;
            Ok(session)
        })
    }

    fn find_by_id<'a>(
        &'a self,
        id: &'a str,
    ) -> BoxFuture<'a, Result<Option<SessionEntity>, ServerError>> {
        Box::pin(async move {
            let row = sqlx::query(
                "SELECT id, owner_id, title, mode, archived_at, draft_content, draft_updated_at, version, created_at, updated_at
                 FROM sessions WHERE id = $1",
            )
            .bind(id)
            .fetch_optional(&self.pool)
            .await
            .map_err(|e| ServerError::Internal(format!("find session failed: {e}")))?;

            row.map(row_to_session).transpose()
        })
    }

    fn list_for_owner<'a>(
        &'a self,
        owner_id: &'a str,
        archived: TriStateFilter<bool>,
        cursor: Option<&'a str>,
        limit: u32,
    ) -> BoxFuture<'a, Result<CursorPage<SessionEntity>, ServerError>> {
        Box::pin(async move {
            let page_size = i64::from(limit.clamp(1, 100));
            let archived_clause: Option<bool> = match archived {
                TriStateFilter::Any => None,
                TriStateFilter::None | TriStateFilter::Exact(false) => Some(false),
                TriStateFilter::Exact(true) => Some(true),
            };
            let (cursor_ts, cursor_id) = parse_cursor(cursor);

            let rows = sqlx::query(
                "SELECT id, owner_id, title, mode, archived_at, draft_content, draft_updated_at, version, created_at, updated_at
                 FROM sessions
                 WHERE owner_id = $1
                   AND ($2::boolean IS NULL OR ($2 = true AND archived_at IS NOT NULL) OR ($2 = false AND archived_at IS NULL))
                   AND ($3::bigint IS NULL OR (updated_at, id) < ($3, $4))
                 ORDER BY updated_at DESC, id DESC
                 LIMIT $5",
            )
            .bind(owner_id)
            .bind(archived_clause)
            .bind(cursor_ts.map(|v| v as i64))
            .bind(cursor_id.unwrap_or_default())
            .bind(page_size + 1)
            .fetch_all(&self.pool)
            .await
            .map_err(|e| ServerError::Internal(format!("list sessions failed: {e}")))?;

            let mut items: Vec<SessionEntity> = rows
                .into_iter()
                .map(row_to_session)
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

    fn update<'a>(
        &'a self,
        session: SessionEntity,
        expected_version: Option<u64>,
    ) -> BoxFuture<'a, Result<SessionEntity, ServerError>> {
        Box::pin(async move {
            let res = sqlx::query(
                "UPDATE sessions
                 SET title = $2, mode = $3, archived_at = $4, version = $5, updated_at = $6
                 WHERE id = $1 AND ($7::bigint IS NULL OR version = $7)",
            )
            .bind(&session.id)
            .bind(&session.title)
            .bind(mode_to_str(session.mode))
            .bind(session.archived_at.map(|v| v as i64))
            .bind(session.version as i64)
            .bind(session.updated_at as i64)
            .bind(expected_version.map(|v| v as i64))
            .execute(&self.pool)
            .await
            .map_err(|e| ServerError::Internal(format!("update session failed: {e}")))?;

            if res.rows_affected() == 0 {
                return Err(ServerError::Conflict(
                    "session modified concurrently or missing".into(),
                ));
            }
            Ok(session)
        })
    }

    fn save_draft<'a>(
        &'a self,
        id: &'a str,
        content: &'a str,
        updated_at: u64,
    ) -> BoxFuture<'a, Result<(), ServerError>> {
        Box::pin(async move {
            sqlx::query(
                "UPDATE sessions SET draft_content = $2, draft_updated_at = $3 WHERE id = $1",
            )
            .bind(id)
            .bind(content)
            .bind(updated_at as i64)
            .execute(&self.pool)
            .await
            .map_err(|e| ServerError::Internal(format!("save draft failed: {e}")))?;
            Ok(())
        })
    }

    fn delete<'a>(&'a self, id: &'a str) -> BoxFuture<'a, Result<bool, ServerError>> {
        Box::pin(async move {
            let res = sqlx::query("DELETE FROM sessions WHERE id = $1")
                .bind(id)
                .execute(&self.pool)
                .await
                .map_err(|e| ServerError::Internal(format!("delete session failed: {e}")))?;
            Ok(res.rows_affected() > 0)
        })
    }
}

fn row_to_session(row: sqlx::postgres::PgRow) -> Result<SessionEntity, ServerError> {
    let mode_str: String = row
        .try_get("mode")
        .map_err(|e| ServerError::Internal(e.to_string()))?;
    let archived_at: Option<i64> = row
        .try_get("archived_at")
        .map_err(|e| ServerError::Internal(e.to_string()))?;
    let draft_updated_at: Option<i64> = row
        .try_get("draft_updated_at")
        .map_err(|e| ServerError::Internal(e.to_string()))?;
    let version: i64 = row
        .try_get("version")
        .map_err(|e| ServerError::Internal(e.to_string()))?;
    let created_at: i64 = row
        .try_get("created_at")
        .map_err(|e| ServerError::Internal(e.to_string()))?;
    let updated_at: i64 = row
        .try_get("updated_at")
        .map_err(|e| ServerError::Internal(e.to_string()))?;

    Ok(SessionEntity {
        id: row
            .try_get("id")
            .map_err(|e| ServerError::Internal(e.to_string()))?,
        owner_id: row
            .try_get("owner_id")
            .map_err(|e| ServerError::Internal(e.to_string()))?,
        title: row
            .try_get("title")
            .map_err(|e| ServerError::Internal(e.to_string()))?,
        mode: str_to_mode(&mode_str),
        archived_at: archived_at.map(|v| v.max(0) as u64),
        draft_content: row
            .try_get("draft_content")
            .map_err(|e| ServerError::Internal(e.to_string()))?,
        draft_updated_at: draft_updated_at.map(|v| v.max(0) as u64),
        version: version.max(1) as u64,
        created_at: created_at.max(0) as u64,
        updated_at: updated_at.max(0) as u64,
    })
}

fn mode_to_str(mode: InteractionMode) -> &'static str {
    match mode {
        InteractionMode::Plan => "plan",
        InteractionMode::Act => "act",
    }
}

fn str_to_mode(raw: &str) -> InteractionMode {
    match raw {
        "plan" => InteractionMode::Plan,
        _ => InteractionMode::Act,
    }
}

fn encode_cursor(session: &SessionEntity) -> String {
    format!("{}:{}", session.updated_at, session.id)
}

fn parse_cursor(cursor: Option<&str>) -> (Option<u64>, Option<&str>) {
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

/// Deterministic in-memory session store used for tests and standalone mode.
#[derive(Default)]
pub struct InMemorySessionRepository {
    sessions: Mutex<Vec<SessionEntity>>,
}

impl InMemorySessionRepository {
    #[must_use]
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }
}

impl SessionRepository for InMemorySessionRepository {
    fn create<'a>(
        &'a self,
        session: SessionEntity,
    ) -> BoxFuture<'a, Result<SessionEntity, ServerError>> {
        Box::pin(async move {
            self.sessions.lock().await.push(session.clone());
            Ok(session)
        })
    }

    fn find_by_id<'a>(
        &'a self,
        id: &'a str,
    ) -> BoxFuture<'a, Result<Option<SessionEntity>, ServerError>> {
        Box::pin(async move {
            Ok(self
                .sessions
                .lock()
                .await
                .iter()
                .find(|s| s.id == id)
                .cloned())
        })
    }

    fn list_for_owner<'a>(
        &'a self,
        owner_id: &'a str,
        archived: TriStateFilter<bool>,
        cursor: Option<&'a str>,
        limit: u32,
    ) -> BoxFuture<'a, Result<CursorPage<SessionEntity>, ServerError>> {
        Box::pin(async move {
            let guard = self.sessions.lock().await;
            let (cursor_ts, cursor_id) = parse_cursor(cursor);
            let mut filtered: Vec<SessionEntity> = guard
                .iter()
                .filter(|s| s.owner_id == owner_id)
                .filter(|s| match archived {
                    TriStateFilter::Any => true,
                    TriStateFilter::None | TriStateFilter::Exact(false) => s.archived_at.is_none(),
                    TriStateFilter::Exact(true) => s.archived_at.is_some(),
                })
                .filter(|s| match (cursor_ts, cursor_id) {
                    (Some(ts), Some(id)) => (s.updated_at, s.id.as_str()) < (ts, id),
                    _ => true,
                })
                .cloned()
                .collect();

            filtered.sort_by(|a, b| (b.updated_at, &b.id).cmp(&(a.updated_at, &a.id)));
            let total_hint = Some(filtered.len() as u64);
            let max_items = limit.clamp(1, 100) as usize;
            let has_more = filtered.len() > max_items;
            filtered.truncate(max_items);
            let next_cursor = if has_more {
                filtered.last().map(encode_cursor)
            } else {
                None
            };

            Ok(CursorPage {
                items: filtered,
                next_cursor,
                backwards_cursor: cursor.map(ToOwned::to_owned),
                total_hint,
            })
        })
    }

    fn update<'a>(
        &'a self,
        session: SessionEntity,
        expected_version: Option<u64>,
    ) -> BoxFuture<'a, Result<SessionEntity, ServerError>> {
        Box::pin(async move {
            let mut guard = self.sessions.lock().await;
            let Some(existing) = guard.iter_mut().find(|s| s.id == session.id) else {
                return Err(ServerError::NotFound(format!(
                    "session `{}` not found",
                    session.id
                )));
            };
            if let Some(expected) = expected_version
                && existing.version != expected
            {
                return Err(ServerError::Conflict(format!(
                    "optimistic version mismatch: expected {expected}, actual {}",
                    existing.version
                )));
            }
            *existing = session.clone();
            Ok(session)
        })
    }

    fn save_draft<'a>(
        &'a self,
        id: &'a str,
        content: &'a str,
        updated_at: u64,
    ) -> BoxFuture<'a, Result<(), ServerError>> {
        Box::pin(async move {
            let mut guard = self.sessions.lock().await;
            let Some(existing) = guard.iter_mut().find(|s| s.id == id) else {
                return Err(ServerError::NotFound(format!("session `{id}` not found")));
            };
            existing.draft_content = Some(content.to_owned());
            existing.draft_updated_at = Some(updated_at);
            Ok(())
        })
    }

    fn delete<'a>(&'a self, id: &'a str) -> BoxFuture<'a, Result<bool, ServerError>> {
        Box::pin(async move {
            let mut guard = self.sessions.lock().await;
            let before = guard.len();
            guard.retain(|s| s.id != id);
            Ok(guard.len() < before)
        })
    }
}
