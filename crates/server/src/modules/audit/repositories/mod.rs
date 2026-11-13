//! Persistence abstraction for audit logs (PostgreSQL + in-memory test repository).

use std::sync::Arc;

use serde_json::Value;
use sqlx::{PgPool, Row};
use tokio::sync::Mutex;

use crate::{
    infra::{error::ServerError, state::BoxFuture},
    modules::audit::entities::AuditLogRecord,
};

pub const GENESIS_AUDIT_HASH: &str =
    "0000000000000000000000000000000000000000000000000000000000000000";

/// Concrete DI wrapper around [`AuditRepository`].
#[derive(Clone)]
pub struct AuditStore(pub Arc<dyn AuditRepository>);

impl Default for AuditStore {
    fn default() -> Self {
        Self(InMemoryAuditRepository::new())
    }
}

/// Persistence operations for append-only hash-chained audit logs.
pub trait AuditRepository: Send + Sync + 'static {
    fn last_hash<'a>(&'a self) -> BoxFuture<'a, Result<String, ServerError>>;
    fn append<'a>(&'a self, entry: AuditLogRecord) -> BoxFuture<'a, Result<(), ServerError>>;
    fn list<'a>(
        &'a self,
        actor_id: Option<&'a str>,
        action: Option<&'a str>,
        limit: u32,
    ) -> BoxFuture<'a, Result<Vec<AuditLogRecord>, ServerError>>;
    fn list_chronological<'a>(&'a self) -> BoxFuture<'a, Result<Vec<AuditLogRecord>, ServerError>>;
    fn purge_older_than<'a>(&'a self, cutoff_ms: u64) -> BoxFuture<'a, Result<u64, ServerError>>;
}

/// PostgreSQL audit log repository.
#[derive(Clone)]
pub struct PgAuditRepository {
    pool: PgPool,
}

impl PgAuditRepository {
    #[must_use]
    pub fn new(pool: PgPool) -> Arc<Self> {
        Arc::new(Self { pool })
    }
}

impl AuditRepository for PgAuditRepository {
    fn last_hash<'a>(&'a self) -> BoxFuture<'a, Result<String, ServerError>> {
        Box::pin(async move {
            let row = sqlx::query(
                "SELECT entry_hash FROM audit_logs ORDER BY created_at DESC, id DESC LIMIT 1",
            )
            .fetch_optional(&self.pool)
            .await
            .map_err(|e| ServerError::Internal(format!("audit last_hash query failed: {e}")))?;

            Ok(row
                .and_then(|r| r.try_get::<String, _>("entry_hash").ok())
                .unwrap_or_else(|| GENESIS_AUDIT_HASH.to_owned()))
        })
    }

    fn append<'a>(&'a self, entry: AuditLogRecord) -> BoxFuture<'a, Result<(), ServerError>> {
        Box::pin(async move {
            sqlx::query(
                "INSERT INTO audit_logs (id, actor_id, action, resource_type, resource_id, outcome, details, prev_hash, entry_hash, created_at)
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)",
            )
            .bind(&entry.id)
            .bind(&entry.actor_id)
            .bind(&entry.action)
            .bind(&entry.resource_type)
            .bind(&entry.resource_id)
            .bind(&entry.outcome)
            .bind(&entry.details)
            .bind(&entry.prev_hash)
            .bind(&entry.entry_hash)
            .bind(entry.created_at as i64)
            .execute(&self.pool)
            .await
            .map_err(|e| ServerError::Internal(format!("audit append failed: {e}")))?;
            Ok(())
        })
    }

    fn list<'a>(
        &'a self,
        actor_id: Option<&'a str>,
        action: Option<&'a str>,
        limit: u32,
    ) -> BoxFuture<'a, Result<Vec<AuditLogRecord>, ServerError>> {
        Box::pin(async move {
            let rows = sqlx::query(
                "SELECT id, actor_id, action, resource_type, resource_id, outcome, details, prev_hash, entry_hash, created_at
                 FROM audit_logs
                 WHERE ($1::text IS NULL OR actor_id = $1)
                   AND ($2::text IS NULL OR action = $2)
                 ORDER BY created_at DESC, id DESC
                 LIMIT $3",
            )
            .bind(actor_id)
            .bind(action)
            .bind(i64::from(limit.clamp(1, 200)))
            .fetch_all(&self.pool)
            .await
            .map_err(|e| ServerError::Internal(format!("audit list failed: {e}")))?;

            rows.into_iter().map(row_to_audit).collect()
        })
    }

    fn list_chronological<'a>(&'a self) -> BoxFuture<'a, Result<Vec<AuditLogRecord>, ServerError>> {
        Box::pin(async move {
            let rows = sqlx::query(
                "SELECT id, actor_id, action, resource_type, resource_id, outcome, details, prev_hash, entry_hash, created_at
                 FROM audit_logs
                 ORDER BY created_at ASC, id ASC",
            )
            .fetch_all(&self.pool)
            .await
            .map_err(|e| ServerError::Internal(format!("audit verify list failed: {e}")))?;

            rows.into_iter().map(row_to_audit).collect()
        })
    }

    fn purge_older_than<'a>(&'a self, cutoff_ms: u64) -> BoxFuture<'a, Result<u64, ServerError>> {
        Box::pin(async move {
            let res = sqlx::query("DELETE FROM audit_logs WHERE created_at < $1")
                .bind(cutoff_ms as i64)
                .execute(&self.pool)
                .await
                .map_err(|e| ServerError::Internal(format!("audit purge failed: {e}")))?;
            Ok(res.rows_affected())
        })
    }
}

fn row_to_audit(row: sqlx::postgres::PgRow) -> Result<AuditLogRecord, ServerError> {
    let created: i64 = row
        .try_get("created_at")
        .map_err(|e| ServerError::Internal(e.to_string()))?;
    Ok(AuditLogRecord {
        id: row
            .try_get("id")
            .map_err(|e| ServerError::Internal(e.to_string()))?,
        actor_id: row
            .try_get("actor_id")
            .map_err(|e| ServerError::Internal(e.to_string()))?,
        action: row
            .try_get("action")
            .map_err(|e| ServerError::Internal(e.to_string()))?,
        resource_type: row
            .try_get("resource_type")
            .map_err(|e| ServerError::Internal(e.to_string()))?,
        resource_id: row
            .try_get("resource_id")
            .map_err(|e| ServerError::Internal(e.to_string()))?,
        outcome: row
            .try_get("outcome")
            .map_err(|e| ServerError::Internal(e.to_string()))?,
        details: row
            .try_get::<Value, _>("details")
            .map_err(|e| ServerError::Internal(e.to_string()))?,
        prev_hash: row
            .try_get("prev_hash")
            .map_err(|e| ServerError::Internal(e.to_string()))?,
        entry_hash: row
            .try_get("entry_hash")
            .map_err(|e| ServerError::Internal(e.to_string()))?,
        created_at: created.max(0) as u64,
    })
}

/// Deterministic in-memory audit repository used in unit/integration tests and standalone mode.
#[derive(Default)]
pub struct InMemoryAuditRepository {
    entries: Mutex<Vec<AuditLogRecord>>,
}

impl InMemoryAuditRepository {
    #[must_use]
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }
}

impl AuditRepository for InMemoryAuditRepository {
    fn last_hash<'a>(&'a self) -> BoxFuture<'a, Result<String, ServerError>> {
        Box::pin(async move {
            let guard = self.entries.lock().await;
            Ok(guard
                .last()
                .map_or_else(|| GENESIS_AUDIT_HASH.to_owned(), |e| e.entry_hash.clone()))
        })
    }

    fn append<'a>(&'a self, entry: AuditLogRecord) -> BoxFuture<'a, Result<(), ServerError>> {
        Box::pin(async move {
            self.entries.lock().await.push(entry);
            Ok(())
        })
    }

    fn list<'a>(
        &'a self,
        actor_id: Option<&'a str>,
        action: Option<&'a str>,
        limit: u32,
    ) -> BoxFuture<'a, Result<Vec<AuditLogRecord>, ServerError>> {
        Box::pin(async move {
            let guard = self.entries.lock().await;
            let items: Vec<AuditLogRecord> = guard
                .iter()
                .rev()
                .filter(|entry| actor_id.is_none_or(|a| entry.actor_id.as_deref() == Some(a)))
                .filter(|entry| action.is_none_or(|act| entry.action == act))
                .take(limit.clamp(1, 200) as usize)
                .cloned()
                .collect();
            Ok(items)
        })
    }

    fn list_chronological<'a>(&'a self) -> BoxFuture<'a, Result<Vec<AuditLogRecord>, ServerError>> {
        Box::pin(async move { Ok(self.entries.lock().await.clone()) })
    }

    fn purge_older_than<'a>(&'a self, cutoff_ms: u64) -> BoxFuture<'a, Result<u64, ServerError>> {
        Box::pin(async move {
            let mut guard = self.entries.lock().await;
            let before = guard.len();
            guard.retain(|e| e.created_at >= cutoff_ms);
            Ok((before - guard.len()) as u64)
        })
    }
}
