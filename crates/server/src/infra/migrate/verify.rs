//! Standalone database migration and audit integrity verification.

use sqlx::{PgPool, Row};

use crate::{
    infra::{error::ServerError, migrate::types::VerifyReport},
    modules::audit::{GENESIS_AUDIT_HASH, compute_entry_hash, entities::AuditLogRecord},
};

/// Validates database migrations and cryptographic audit hash continuity.
///
/// # Errors
///
/// Returns [`ServerError`] if database queries fail.
pub async fn verify_database(pool: &PgPool) -> Result<VerifyReport, ServerError> {
    let applied_migrations: i64 =
        sqlx::query_scalar("SELECT count(*) FROM _sqlx_migrations WHERE success = true")
            .fetch_one(pool)
            .await
            .unwrap_or(0);

    let rows = sqlx::query(
        "SELECT id, actor_id, action, resource_type, resource_id, outcome, details, prev_hash, entry_hash, created_at FROM audit_logs ORDER BY created_at ASC, id ASC"
    )
    .fetch_all(pool)
    .await
    .map_err(|e| ServerError::Internal(format!("Failed to query audit logs for verification: {e}")))?;

    let total_audit_logs = rows.len();
    let mut expected_prev = GENESIS_AUDIT_HASH.to_owned();

    for row in rows {
        let entry = AuditLogRecord {
            id: row.try_get("id").map_err(err_map)?,
            actor_id: row.try_get("actor_id").unwrap_or(None),
            action: row.try_get("action").map_err(err_map)?,
            resource_type: row.try_get("resource_type").map_err(err_map)?,
            resource_id: row.try_get("resource_id").unwrap_or(None),
            outcome: row.try_get("outcome").map_err(err_map)?,
            details: row.try_get("details").map_err(err_map)?,
            prev_hash: row.try_get("prev_hash").map_err(err_map)?,
            entry_hash: row.try_get("entry_hash").map_err(err_map)?,
            created_at: u64::try_from(row.try_get::<i64, _>("created_at").unwrap_or(0).max(0))
                .unwrap_or(0),
        };

        if entry.prev_hash != expected_prev {
            return Ok(VerifyReport {
                healthy: false,
                applied_migrations: usize::try_from(applied_migrations).unwrap_or(0),
                total_audit_logs,
                audit_chain_valid: false,
                broken_entry_id: Some(entry.id.clone()),
                message: format!(
                    "Audit log chain broken at entry {}: expected prev_hash {}, found {}",
                    entry.id, expected_prev, entry.prev_hash
                ),
            });
        }

        let recomputed = compute_entry_hash(
            &entry.prev_hash,
            &entry.id,
            entry.actor_id.as_deref(),
            &entry.action,
            &entry.resource_type,
            entry.resource_id.as_deref(),
            &entry.outcome,
            &entry.details,
            entry.created_at,
        );

        if recomputed != entry.entry_hash {
            return Ok(VerifyReport {
                healthy: false,
                applied_migrations: usize::try_from(applied_migrations).unwrap_or(0),
                total_audit_logs,
                audit_chain_valid: false,
                broken_entry_id: Some(entry.id.clone()),
                message: format!(
                    "Audit log hash mismatch at entry {}: expected {}, recomputed {}",
                    entry.id, entry.entry_hash, recomputed
                ),
            });
        }

        expected_prev = entry.entry_hash;
    }

    Ok(VerifyReport {
        healthy: true,
        applied_migrations: usize::try_from(applied_migrations).unwrap_or(0),
        total_audit_logs,
        audit_chain_valid: true,
        broken_entry_id: None,
        message: format!(
            "Database verified healthy: {applied_migrations} migrations applied, {total_audit_logs} audit entries verified"
        ),
    })
}

fn err_map(err: sqlx::Error) -> ServerError {
    ServerError::Internal(format!("Database error: {err}"))
}
