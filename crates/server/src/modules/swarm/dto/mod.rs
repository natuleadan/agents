//! Validated DTOs for swarm subagent handoffs, signed messaging, and blackboard entries.

use agents_protocol::PermissionMask;
use garde::Validate;
use ironic::OpenApiSchema;
use serde::Deserialize;
use serde_json::Value;

/// Request to spawn a child subagent delegation constrained to a subset of parent permissions.
#[derive(Debug, Clone, Deserialize, Validate, OpenApiSchema)]
#[serde(rename_all = "camelCase")]
pub struct CreateHandoffDto {
    #[garde(length(min = 1, max = 128))]
    pub parent_agent_id: String,
    #[garde(length(min = 1, max = 128))]
    pub child_agent_id: String,
    #[garde(skip)]
    pub parent_mask: PermissionMask,
    #[garde(skip)]
    pub requested_mask: PermissionMask,
    #[garde(length(max = 128))]
    pub worktree_branch: Option<String>,
    #[garde(length(min = 1, max = 4096))]
    pub goal: String,
}

/// Request to dispatch a signed NATS swarm message or mediated user <-> subagent message.
#[derive(Debug, Clone, Deserialize, Validate)]
#[serde(rename_all = "camelCase")]
pub struct SendSwarmMessageDto {
    #[garde(length(min = 1, max = 128))]
    pub from_agent_id: String,
    #[garde(length(max = 128))]
    pub target_agent_id: Option<String>,
    #[garde(skip)]
    pub permission_mask: PermissionMask,
    #[garde(skip)]
    pub payload: Value,
    #[garde(length(max = 256))]
    pub signature: Option<String>,
}

impl OpenApiSchema for SendSwarmMessageDto {
    fn openapi_schema() -> Value {
        serde_json::json!({
            "type": "object",
            "title": "SendSwarmMessageDto",
            "required": ["fromAgentId", "permissionMask", "payload"],
            "properties": {
                "fromAgentId": { "type": "string", "example": "agent-leader" },
                "targetAgentId": { "type": "string", "nullable": true, "example": "agent-worker-01" },
                "permissionMask": PermissionMask::openapi_schema(),
                "payload": { "type": "object", "example": { "instruction": "Process chunk A" } },
                "signature": { "type": "string", "nullable": true, "example": "hmac-sha256-signature" }
            }
        })
    }
}

/// Request to write a shared entry onto a session's Valkey blackboard.
#[derive(Debug, Clone, Deserialize, Validate)]
#[serde(rename_all = "camelCase")]
pub struct PutBlackboardDto {
    #[garde(length(min = 1, max = 128))]
    pub key: String,
    #[garde(skip)]
    pub value: Value,
}

impl OpenApiSchema for PutBlackboardDto {
    fn openapi_schema() -> Value {
        serde_json::json!({
            "type": "object",
            "title": "PutBlackboardDto",
            "required": ["key", "value"],
            "properties": {
                "key": { "type": "string", "example": "shared_progress" },
                "value": { "type": "object", "example": { "completedSteps": 3 } }
            }
        })
    }
}

/// Response payload representing a stored or retrieved shared blackboard key-value entry.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BlackboardEntryResponse {
    pub key: String,
    pub value: Value,
}

impl OpenApiSchema for BlackboardEntryResponse {
    fn openapi_schema() -> Value {
        serde_json::json!({
            "type": "object",
            "title": "BlackboardEntryResponse",
            "required": ["key", "value"],
            "properties": {
                "key": { "type": "string", "example": "shared_progress" },
                "value": { "type": "object", "example": { "completedSteps": 3 } }
            }
        })
    }
}
