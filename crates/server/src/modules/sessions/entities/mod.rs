//! Session and draft recovery domain entities.

use agents_protocol::InteractionMode;
use ironic::OpenApiSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Persisted conversation/sandbox session owned by a user.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionEntity {
    pub id: String,
    pub owner_id: String,
    pub title: String,
    pub mode: InteractionMode,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub archived_at: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub draft_content: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub draft_updated_at: Option<u64>,
    pub version: u64,
    pub created_at: u64,
    pub updated_at: u64,
}

impl OpenApiSchema for SessionEntity {
    fn openapi_schema() -> Value {
        serde_json::json!({
            "type": "object",
            "title": "SessionEntity",
            "required": ["id", "ownerId", "title", "mode", "version", "createdAt", "updatedAt"],
            "properties": {
                "id": { "type": "string", "example": "ses_01j7sess001" },
                "ownerId": { "type": "string", "example": "usr_01j7abc" },
                "title": { "type": "string", "example": "Web Scraper Orchestration" },
                "mode": InteractionMode::openapi_schema(),
                "archivedAt": { "type": "integer", "nullable": true },
                "draftContent": { "type": "string", "nullable": true },
                "draftUpdatedAt": { "type": "integer", "nullable": true },
                "version": { "type": "integer", "example": 1 },
                "createdAt": { "type": "integer", "example": 1731513600 },
                "updatedAt": { "type": "integer", "example": 1731513600 }
            }
        })
    }
}

/// Recovered unsent prompt draft for crash-resilient client UX.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, OpenApiSchema)]
#[serde(rename_all = "camelCase")]
pub struct DraftSnapshot {
    pub session_id: String,
    pub content: Option<String>,
    pub updated_at: Option<u64>,
}
