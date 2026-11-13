//! Domain entity for tamper-evident hash-chained audit logs.

use ironic::OpenApiSchema;
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

impl OpenApiSchema for AuditLogRecord {
    fn openapi_schema() -> Value {
        serde_json::json!({
            "type": "object",
            "title": "AuditLogRecord",
            "required": ["id", "action", "resourceType", "outcome", "details", "prevHash", "entryHash", "createdAt"],
            "properties": {
                "id": { "type": "string", "example": "aud_01j7xyz" },
                "actorId": { "type": "string", "nullable": true, "example": "usr_01j7abc" },
                "action": { "type": "string", "example": "tasks.submit" },
                "resourceType": { "type": "string", "example": "task" },
                "resourceId": { "type": "string", "nullable": true, "example": "tsk_01j7task001" },
                "outcome": { "type": "string", "example": "allowed" },
                "details": { "type": "object", "example": { "clientIp": "127.0.0.1" } },
                "prevHash": { "type": "string", "example": "0000000000000000000000000000000000000000000000000000000000000000" },
                "entryHash": { "type": "string", "example": "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855" },
                "createdAt": { "type": "integer", "example": 1731513600 }
            }
        })
    }
}
