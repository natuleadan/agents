//! Database snapshot export engine preserving cryptographic audit hash chains.

use sqlx::{PgPool, Row};

use crate::{
    infra::{
        error::ServerError,
        migrate::types::{
            MigrateApiKeyRecord, MigrateAutocronRecord, MigrateRenderedSlugRecord,
            MigrateSessionRecord, MigrateUserRecord, MigrateWorkflowRecord, MigrationSnapshot,
        },
        state::now_ms,
    },
    modules::audit::{GENESIS_AUDIT_HASH, compute_entry_hash, entities::AuditLogRecord},
};

/// Dumps all core entities and tamper-evident audit logs into a versioned [`MigrationSnapshot`].
///
/// # Errors
///
/// Returns [`ServerError`] if database queries fail or cryptographic hash verification fails.
pub async fn export_snapshot(
    pool: &PgPool,
    source_mode: &str,
) -> Result<MigrationSnapshot, ServerError> {
    let users = fetch_users(pool).await?;
    let api_keys = fetch_api_keys(pool).await?;
    let sessions = fetch_sessions(pool).await?;
    let workflows = fetch_workflows(pool).await?;
    let autocron_schedules = fetch_autocron_schedules(pool).await?;
    let rendered_slugs = fetch_rendered_slugs(pool).await?;
    let audit_logs = fetch_audit_logs(pool).await?;

    let (valid, head_hash) = validate_in_memory_audit_chain(&audit_logs);
    if !valid {
        return Err(ServerError::Internal(
            "Source database audit log chain is compromised; export aborted".to_owned(),
        ));
    }

    Ok(MigrationSnapshot {
        version: 1,
        exported_at: now_ms(),
        source_mode: source_mode.to_owned(),
        users,
        api_keys,
        sessions,
        workflows,
        autocron_schedules,
        rendered_slugs,
        audit_logs,
        audit_hash_chain_valid: valid,
        audit_head_hash: head_hash,
    })
}

async fn fetch_users(pool: &PgPool) -> Result<Vec<MigrateUserRecord>, ServerError> {
    let rows = sqlx::query(
        "SELECT id, email, password_hash, role, created_at FROM users ORDER BY created_at ASC",
    )
    .fetch_all(pool)
    .await
    .map_err(|e| ServerError::Internal(format!("Failed to fetch users: {e}")))?;

    rows.into_iter()
        .map(|row| {
            let created_at_i64: i64 = row.try_get("created_at").unwrap_or(0);
            Ok(MigrateUserRecord {
                id: row.try_get("id").map_err(err_map)?,
                email: row.try_get("email").map_err(err_map)?,
                password_hash: row.try_get("password_hash").map_err(err_map)?,
                role: row.try_get("role").map_err(err_map)?,
                created_at: u64::try_from(created_at_i64.max(0)).unwrap_or(0),
            })
        })
        .collect()
}

async fn fetch_api_keys(pool: &PgPool) -> Result<Vec<MigrateApiKeyRecord>, ServerError> {
    let rows = sqlx::query("SELECT id, user_id, name, key_hash, scopes, expires_at, created_at FROM api_keys ORDER BY created_at ASC")
        .fetch_all(pool)
        .await
        .map_err(|e| ServerError::Internal(format!("Failed to fetch api keys: {e}")))?;

    rows.into_iter()
        .map(|row| {
            let created_at_i64: i64 = row.try_get("created_at").unwrap_or(0);
            let expires_at_i64: Option<i64> = row.try_get("expires_at").unwrap_or(None);
            Ok(MigrateApiKeyRecord {
                id: row.try_get("id").map_err(err_map)?,
                user_id: row.try_get("user_id").map_err(err_map)?,
                name: row.try_get("name").map_err(err_map)?,
                key_hash: row.try_get("key_hash").map_err(err_map)?,
                scopes: row.try_get("scopes").map_err(err_map)?,
                expires_at: expires_at_i64.and_then(|t| u64::try_from(t).ok()),
                created_at: u64::try_from(created_at_i64.max(0)).unwrap_or(0),
            })
        })
        .collect()
}

async fn fetch_sessions(pool: &PgPool) -> Result<Vec<MigrateSessionRecord>, ServerError> {
    let rows = sqlx::query("SELECT id, user_id, title, metadata, created_at, updated_at FROM sessions ORDER BY created_at ASC")
        .fetch_all(pool)
        .await
        .map_err(|e| ServerError::Internal(format!("Failed to fetch sessions: {e}")))?;

    rows.into_iter()
        .map(|row| {
            let created_at_i64: i64 = row.try_get("created_at").unwrap_or(0);
            let updated_at_i64: i64 = row.try_get("updated_at").unwrap_or(0);
            Ok(MigrateSessionRecord {
                id: row.try_get("id").map_err(err_map)?,
                user_id: row.try_get("user_id").map_err(err_map)?,
                title: row.try_get("title").map_err(err_map)?,
                metadata: row.try_get("metadata").map_err(err_map)?,
                created_at: u64::try_from(created_at_i64.max(0)).unwrap_or(0),
                updated_at: u64::try_from(updated_at_i64.max(0)).unwrap_or(0),
            })
        })
        .collect()
}

async fn fetch_workflows(pool: &PgPool) -> Result<Vec<MigrateWorkflowRecord>, ServerError> {
    let rows = sqlx::query(
        "SELECT id, creator_id, name, description, dag, is_active, approval_role, approved_at, approved_by, created_at, updated_at FROM workflows ORDER BY created_at ASC"
    )
    .fetch_all(pool)
    .await
    .map_err(|e| ServerError::Internal(format!("Failed to fetch workflows: {e}")))?;

    rows.into_iter()
        .map(|row| {
            let created_at_i64: i64 = row.try_get("created_at").unwrap_or(0);
            let updated_at_i64: i64 = row.try_get("updated_at").unwrap_or(0);
            let approved_at_i64: Option<i64> = row.try_get("approved_at").unwrap_or(None);
            Ok(MigrateWorkflowRecord {
                id: row.try_get("id").map_err(err_map)?,
                creator_id: row.try_get("creator_id").map_err(err_map)?,
                name: row.try_get("name").map_err(err_map)?,
                description: row.try_get("description").map_err(err_map)?,
                dag: row.try_get("dag").map_err(err_map)?,
                is_active: row.try_get("is_active").map_err(err_map)?,
                approval_role: row.try_get("approval_role").unwrap_or(None),
                approved_at: approved_at_i64.and_then(|t| u64::try_from(t).ok()),
                approved_by: row.try_get("approved_by").unwrap_or(None),
                created_at: u64::try_from(created_at_i64.max(0)).unwrap_or(0),
                updated_at: u64::try_from(updated_at_i64.max(0)).unwrap_or(0),
            })
        })
        .collect()
}

async fn fetch_autocron_schedules(
    pool: &PgPool,
) -> Result<Vec<MigrateAutocronRecord>, ServerError> {
    let rows = sqlx::query(
        "SELECT id, workflow_id, schedule_cron, description, is_active, max_jitter_secs, created_at, updated_at FROM autocron_schedules ORDER BY created_at ASC"
    )
    .fetch_all(pool)
    .await
    .map_err(|e| ServerError::Internal(format!("Failed to fetch autocron schedules: {e}")))?;

    rows.into_iter()
        .map(|row| {
            let created_at_i64: i64 = row.try_get("created_at").unwrap_or(0);
            let updated_at_i64: i64 = row.try_get("updated_at").unwrap_or(0);
            let max_jitter_i64: i64 = row.try_get("max_jitter_secs").unwrap_or(0);
            Ok(MigrateAutocronRecord {
                id: row.try_get("id").map_err(err_map)?,
                workflow_id: row.try_get("workflow_id").map_err(err_map)?,
                schedule_cron: row.try_get("schedule_cron").map_err(err_map)?,
                description: row.try_get("description").map_err(err_map)?,
                is_active: row.try_get("is_active").map_err(err_map)?,
                max_jitter_secs: u64::try_from(max_jitter_i64.max(0)).unwrap_or(0),
                created_at: u64::try_from(created_at_i64.max(0)).unwrap_or(0),
                updated_at: u64::try_from(updated_at_i64.max(0)).unwrap_or(0),
            })
        })
        .collect()
}

async fn fetch_rendered_slugs(
    pool: &PgPool,
) -> Result<Vec<MigrateRenderedSlugRecord>, ServerError> {
    let rows = sqlx::query(
        "SELECT slug, workflow_id, run_id, content_type, rendered_body, published_by, published_at, expires_at FROM rendered_slugs ORDER BY published_at ASC"
    )
    .fetch_all(pool)
    .await
    .map_err(|e| ServerError::Internal(format!("Failed to fetch rendered slugs: {e}")))?;

    rows.into_iter()
        .map(|row| {
            let published_at_i64: i64 = row.try_get("published_at").unwrap_or(0);
            let expires_at_i64: Option<i64> = row.try_get("expires_at").unwrap_or(None);
            Ok(MigrateRenderedSlugRecord {
                slug: row.try_get("slug").map_err(err_map)?,
                workflow_id: row.try_get("workflow_id").map_err(err_map)?,
                run_id: row.try_get("run_id").map_err(err_map)?,
                content_type: row.try_get("content_type").map_err(err_map)?,
                rendered_body: row.try_get("rendered_body").map_err(err_map)?,
                published_by: row.try_get("published_by").map_err(err_map)?,
                published_at: u64::try_from(published_at_i64.max(0)).unwrap_or(0),
                expires_at: expires_at_i64.and_then(|t| u64::try_from(t).ok()),
            })
        })
        .collect()
}

async fn fetch_audit_logs(pool: &PgPool) -> Result<Vec<AuditLogRecord>, ServerError> {
    let rows = sqlx::query(
        "SELECT id, actor_id, action, resource_type, resource_id, outcome, details, prev_hash, entry_hash, created_at FROM audit_logs ORDER BY created_at ASC, id ASC"
    )
    .fetch_all(pool)
    .await
    .map_err(|e| ServerError::Internal(format!("Failed to fetch audit logs: {e}")))?;

    rows.into_iter()
        .map(|row| {
            let created_at_i64: i64 = row.try_get("created_at").unwrap_or(0);
            Ok(AuditLogRecord {
                id: row.try_get("id").map_err(err_map)?,
                actor_id: row.try_get("actor_id").unwrap_or(None),
                action: row.try_get("action").map_err(err_map)?,
                resource_type: row.try_get("resource_type").map_err(err_map)?,
                resource_id: row.try_get("resource_id").unwrap_or(None),
                outcome: row.try_get("outcome").map_err(err_map)?,
                details: row.try_get("details").map_err(err_map)?,
                prev_hash: row.try_get("prev_hash").map_err(err_map)?,
                entry_hash: row.try_get("entry_hash").map_err(err_map)?,
                created_at: u64::try_from(created_at_i64.max(0)).unwrap_or(0),
            })
        })
        .collect()
}

/// Validates an in-memory audit log sequence, verifying genesis continuity and SHA-256 hashes.
#[must_use]
pub fn validate_in_memory_audit_chain(records: &[AuditLogRecord]) -> (bool, String) {
    let mut expected_prev = GENESIS_AUDIT_HASH.to_owned();
    for entry in records {
        if entry.prev_hash != expected_prev {
            return (false, String::new());
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
            return (false, String::new());
        }
        expected_prev = entry.entry_hash.clone();
    }
    (true, expected_prev)
}

fn err_map(err: sqlx::Error) -> ServerError {
    ServerError::Internal(format!("Database row decoding error: {err}"))
}
