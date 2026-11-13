//! Domain entity for tamper-evident hash-chained audit logs.

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Recorded security/lifecycle event chained by SHA-256 hash to the previous entry.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AuditLogRecord {
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub actor_id: Option<String>,
    pub action: String,
    pub resource_type: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resource_id: Option<String>,
    pub outcome: String,
    pub details: Value,
    pub prev_hash: String,
    pub entry_hash: String,
    pub created_at: u64,
}
