//! Database snapshot import engine with preflight validation and atomic restoration.

use sqlx::{PgPool, Row};

use crate::{
    infra::{
        error::ServerError,
        migrate::{
            export::validate_in_memory_audit_chain,
            types::{ImportSummary, MigrationSnapshot},
        },
    },
    modules::audit::{GENESIS_AUDIT_HASH, compute_entry_hash, entities::AuditLogRecord},
};

/// Restores a [`MigrationSnapshot`] into the target PostgreSQL/Neon/Supabase database.
///
/// # Errors
///
/// Returns [`ServerError`] if preflight validation fails, migration fails, or import fails.
pub async fn import_snapshot(
    pool: &PgPool,
    snapshot: &MigrationSnapshot,
) -> Result<ImportSummary, ServerError> {
    // 1. In-memory tamper check before touching target database
    let (valid, _head) = validate_in_memory_audit_chain(&snapshot.audit_logs);
    if !valid {
        return Err(ServerError::Internal(
            "Snapshot verification failed: audit hash chain is corrupted or tampered".to_owned(),
        ));
    }

    // 2. Ensure target database schema migrations are applied
    sqlx::migrate!("./migrations")
        .run(pool)
        .await
        .map_err(|e| ServerError::Internal(format!("Failed to apply target migrations: {e}")))?;

    // 3. Begin atomic transaction
    let mut tx = pool
        .begin()
        .await
        .map_err(|e| ServerError::Internal(format!("Failed to begin transaction: {e}")))?;

    for u in &snapshot.users {
        sqlx::query(
            r#"INSERT INTO users (id, email, password_hash, role, created_at)
               VALUES ($1, $2, $3, $4, $5)
               ON CONFLICT (id) DO UPDATE SET
                   email = EXCLUDED.email,
                   password_hash = EXCLUDED.password_hash,
                   role = EXCLUDED.role"#,
        )
        .bind(&u.id)
        .bind(&u.email)
        .bind(&u.password_hash)
        .bind(&u.role)
        .bind(i64::try_from(u.created_at).unwrap_or(0))
        .execute(&mut *tx)
        .await
        .map_err(|e| ServerError::Internal(format!("Failed to insert user {}: {e}", u.id)))?;
    }

    for k in &snapshot.api_keys {
        sqlx::query(
            r#"INSERT INTO api_keys (id, user_id, name, key_hash, scopes, expires_at, created_at)
               VALUES ($1, $2, $3, $4, $5, $6, $7)
               ON CONFLICT (id) DO UPDATE SET
                   name = EXCLUDED.name,
                   key_hash = EXCLUDED.key_hash,
                   scopes = EXCLUDED.scopes,
                   expires_at = EXCLUDED.expires_at"#,
        )
        .bind(&k.id)
        .bind(&k.user_id)
        .bind(&k.name)
        .bind(&k.key_hash)
        .bind(&k.scopes)
        .bind(k.expires_at.and_then(|t| i64::try_from(t).ok()))
        .bind(i64::try_from(k.created_at).unwrap_or(0))
        .execute(&mut *tx)
        .await
        .map_err(|e| ServerError::Internal(format!("Failed to insert api key {}: {e}", k.id)))?;
    }

    for s in &snapshot.sessions {
        sqlx::query(
            r#"INSERT INTO sessions (id, user_id, title, metadata, created_at, updated_at)
               VALUES ($1, $2, $3, $4, $5, $6)
               ON CONFLICT (id) DO UPDATE SET
                   title = EXCLUDED.title,
                   metadata = EXCLUDED.metadata,
                   updated_at = EXCLUDED.updated_at"#,
        )
        .bind(&s.id)
        .bind(&s.user_id)
        .bind(&s.title)
        .bind(&s.metadata)
        .bind(i64::try_from(s.created_at).unwrap_or(0))
        .bind(i64::try_from(s.updated_at).unwrap_or(0))
        .execute(&mut *tx)
        .await
        .map_err(|e| ServerError::Internal(format!("Failed to insert session {}: {e}", s.id)))?;
    }

    for w in &snapshot.workflows {
        sqlx::query(
            r#"INSERT INTO workflows (id, creator_id, name, description, dag, is_active, approval_role, approved_at, approved_by, created_at, updated_at)
               VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)
               ON CONFLICT (id) DO UPDATE SET
                   name = EXCLUDED.name,
                   description = EXCLUDED.description,
                   dag = EXCLUDED.dag,
                   is_active = EXCLUDED.is_active,
                   approval_role = EXCLUDED.approval_role,
                   approved_at = EXCLUDED.approved_at,
                   approved_by = EXCLUDED.approved_by,
                   updated_at = EXCLUDED.updated_at"#
        )
        .bind(&w.id)
        .bind(&w.creator_id)
        .bind(&w.name)
        .bind(&w.description)
        .bind(&w.dag)
        .bind(w.is_active)
        .bind(&w.approval_role)
        .bind(w.approved_at.and_then(|t| i64::try_from(t).ok()))
        .bind(&w.approved_by)
        .bind(i64::try_from(w.created_at).unwrap_or(0))
        .bind(i64::try_from(w.updated_at).unwrap_or(0))
        .execute(&mut *tx)
        .await
        .map_err(|e| ServerError::Internal(format!("Failed to insert workflow {}: {e}", w.id)))?;
    }

    for c in &snapshot.autocron_schedules {
        sqlx::query(
            r#"INSERT INTO autocron_schedules (id, workflow_id, schedule_cron, description, is_active, max_jitter_secs, created_at, updated_at)
               VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
               ON CONFLICT (id) DO UPDATE SET
                   schedule_cron = EXCLUDED.schedule_cron,
                   description = EXCLUDED.description,
                   is_active = EXCLUDED.is_active,
                   max_jitter_secs = EXCLUDED.max_jitter_secs,
                   updated_at = EXCLUDED.updated_at"#
        )
        .bind(&c.id)
        .bind(&c.workflow_id)
        .bind(&c.schedule_cron)
        .bind(&c.description)
        .bind(c.is_active)
        .bind(i64::try_from(c.max_jitter_secs).unwrap_or(0))
        .bind(i64::try_from(c.created_at).unwrap_or(0))
        .bind(i64::try_from(c.updated_at).unwrap_or(0))
        .execute(&mut *tx)
        .await
        .map_err(|e| ServerError::Internal(format!("Failed to insert autocron {}: {e}", c.id)))?;
    }

    for r in &snapshot.rendered_slugs {
        sqlx::query(
            r#"INSERT INTO rendered_slugs (slug, workflow_id, run_id, content_type, rendered_body, published_by, published_at, expires_at)
               VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
               ON CONFLICT (slug) DO UPDATE SET
                   workflow_id = EXCLUDED.workflow_id,
                   run_id = EXCLUDED.run_id,
                   content_type = EXCLUDED.content_type,
                   rendered_body = EXCLUDED.rendered_body,
                   published_by = EXCLUDED.published_by,
                   published_at = EXCLUDED.published_at,
                   expires_at = EXCLUDED.expires_at"#
        )
        .bind(&r.slug)
        .bind(&r.workflow_id)
        .bind(&r.run_id)
        .bind(&r.content_type)
        .bind(&r.rendered_body)
        .bind(&r.published_by)
        .bind(i64::try_from(r.published_at).unwrap_or(0))
        .bind(r.expires_at.and_then(|t| i64::try_from(t).ok()))
        .execute(&mut *tx)
        .await
        .map_err(|e| ServerError::Internal(format!("Failed to insert rendered slug {}: {e}", r.slug)))?;
    }

    for a in &snapshot.audit_logs {
        sqlx::query(
            r#"INSERT INTO audit_logs (id, actor_id, action, resource_type, resource_id, outcome, details, prev_hash, entry_hash, created_at)
               VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)
               ON CONFLICT (id) DO NOTHING"#
        )
        .bind(&a.id)
        .bind(&a.actor_id)
        .bind(&a.action)
        .bind(&a.resource_type)
        .bind(&a.resource_id)
        .bind(&a.outcome)
        .bind(&a.details)
        .bind(&a.prev_hash)
        .bind(&a.entry_hash)
        .bind(i64::try_from(a.created_at).unwrap_or(0))
        .execute(&mut *tx)
        .await
        .map_err(|e| ServerError::Internal(format!("Failed to insert audit log {}: {e}", a.id)))?;
    }

    tx.commit()
        .await
        .map_err(|e| ServerError::Internal(format!("Transaction commit failed: {e}")))?;

    // 4. Verify cryptographic chain in target database after commit
    let (target_valid, _head) = verify_target_audit_chain(pool).await?;
    if !target_valid {
        return Err(ServerError::Internal(
            "Target database audit verification failed after import".to_owned(),
        ));
    }

    Ok(ImportSummary {
        users_imported: snapshot.users.len(),
        api_keys_imported: snapshot.api_keys.len(),
        sessions_imported: snapshot.sessions.len(),
        workflows_imported: snapshot.workflows.len(),
        autocron_schedules_imported: snapshot.autocron_schedules.len(),
        rendered_slugs_imported: snapshot.rendered_slugs.len(),
        audit_logs_imported: snapshot.audit_logs.len(),
        audit_chain_valid: target_valid,
    })
}

async fn verify_target_audit_chain(pool: &PgPool) -> Result<(bool, String), ServerError> {
    let rows = sqlx::query(
        "SELECT id, actor_id, action, resource_type, resource_id, outcome, details, prev_hash, entry_hash, created_at FROM audit_logs ORDER BY created_at ASC, id ASC"
    )
    .fetch_all(pool)
    .await
    .map_err(|e| ServerError::Internal(format!("Failed to query target audit logs: {e}")))?;

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
            return Ok((false, String::new()));
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
            return Ok((false, String::new()));
        }
        expected_prev = entry.entry_hash;
    }
    Ok((true, expected_prev))
}

fn err_map(err: sqlx::Error) -> ServerError {
    ServerError::Internal(format!("Database decoding error: {err}"))
}
