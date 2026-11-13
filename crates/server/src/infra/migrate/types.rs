//! Serializable models and checksum envelopes for database migrations.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::modules::audit::entities::AuditLogRecord;

/// Versioned snapshot containing exported entities and tamper-evident audit logs.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MigrationSnapshot {
    pub version: u32,
    pub exported_at: u64,
    pub source_mode: String,
    pub users: Vec<MigrateUserRecord>,
    pub api_keys: Vec<MigrateApiKeyRecord>,
    pub sessions: Vec<MigrateSessionRecord>,
    pub workflows: Vec<MigrateWorkflowRecord>,
    pub autocron_schedules: Vec<MigrateAutocronRecord>,
    pub rendered_slugs: Vec<MigrateRenderedSlugRecord>,
    pub audit_logs: Vec<AuditLogRecord>,
    pub audit_hash_chain_valid: bool,
    pub audit_head_hash: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MigrateUserRecord {
    pub id: String,
    pub email: String,
    pub password_hash: String,
    pub role: String,
    pub created_at: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MigrateApiKeyRecord {
    pub id: String,
    pub user_id: String,
    pub name: String,
    pub key_hash: String,
    pub scopes: Value,
    pub expires_at: Option<u64>,
    pub created_at: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MigrateSessionRecord {
    pub id: String,
    pub user_id: String,
    pub title: String,
    pub metadata: Value,
    pub created_at: u64,
    pub updated_at: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MigrateWorkflowRecord {
    pub id: String,
    pub creator_id: String,
    pub name: String,
    pub description: String,
    pub dag: Value,
    pub is_active: bool,
    pub approval_role: Option<String>,
    pub approved_at: Option<u64>,
    pub approved_by: Option<String>,
    pub created_at: u64,
    pub updated_at: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MigrateAutocronRecord {
    pub id: String,
    pub workflow_id: String,
    pub schedule_cron: String,
    pub description: String,
    pub is_active: bool,
    pub max_jitter_secs: u64,
    pub created_at: u64,
    pub updated_at: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MigrateRenderedSlugRecord {
    pub slug: String,
    pub workflow_id: String,
    pub run_id: String,
    pub content_type: String,
    pub rendered_body: String,
    pub published_by: String,
    pub published_at: u64,
    pub expires_at: Option<u64>,
}

/// Summary report after importing a snapshot into the target database.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImportSummary {
    pub users_imported: usize,
    pub api_keys_imported: usize,
    pub sessions_imported: usize,
    pub workflows_imported: usize,
    pub autocron_schedules_imported: usize,
    pub rendered_slugs_imported: usize,
    pub audit_logs_imported: usize,
    pub audit_chain_valid: bool,
}

/// Diagnostic report verifying target database migration status and audit chain.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VerifyReport {
    pub healthy: bool,
    pub applied_migrations: usize,
    pub total_audit_logs: usize,
    pub audit_chain_valid: bool,
    pub broken_entry_id: Option<String>,
    pub message: String,
}
