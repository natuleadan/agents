//! Validated DTOs for swarm subagent handoffs, signed messaging, and blackboard entries.

use agents_protocol::PermissionMask;
use garde::Validate;
use serde::Deserialize;
use serde_json::Value;

/// Request to spawn a child subagent delegation constrained to a subset of parent permissions.
#[derive(Debug, Clone, Deserialize, Validate)]
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

/// Request to write a shared entry onto a session's Valkey blackboard.
#[derive(Debug, Clone, Deserialize, Validate)]
#[serde(rename_all = "camelCase")]
pub struct PutBlackboardDto {
    #[garde(length(min = 1, max = 128))]
    pub key: String,
    #[garde(skip)]
    pub value: Value,
}
