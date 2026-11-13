//! Session and draft recovery domain entities.

use agents_protocol::InteractionMode;
use ironic::OpenApiSchema;
use serde::{Deserialize, Serialize};

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

/// Recovered unsent prompt draft for crash-resilient client UX.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, OpenApiSchema)]
#[serde(rename_all = "camelCase")]
pub struct DraftSnapshot {
    pub session_id: String,
    pub content: Option<String>,
    pub updated_at: Option<u64>,
}
