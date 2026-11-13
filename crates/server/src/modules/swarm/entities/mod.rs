//! Swarm domain entities representing delegated subagents and mediated messages.

use agents_protocol::{AgentHandoff, PermissionMask};
use ironic::OpenApiSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Stored subagent delegation record enforcing narrowed OWASP ASI-07 bounds.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SwarmHandoffEntity {
    pub handoff_id: String,
    pub session_id: String,
    pub owner_id: String,
    pub parent_agent_id: String,
    pub child_agent_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub worktree_branch: Option<String>,
    pub delegated_mask: PermissionMask,
    pub goal: String,
    pub created_at: u64,
}

impl OpenApiSchema for SwarmHandoffEntity {
    fn openapi_schema() -> Value {
        serde_json::json!({
            "type": "object",
            "title": "SwarmHandoffEntity",
            "required": ["handoffId", "sessionId", "ownerId", "parentAgentId", "childAgentId", "delegatedMask", "goal", "createdAt"],
            "properties": {
                "handoffId": { "type": "string", "example": "hnd_01j7xyz" },
                "sessionId": { "type": "string", "example": "ses_01j7sess" },
                "ownerId": { "type": "string", "example": "usr_01j7abc" },
                "parentAgentId": { "type": "string", "example": "leader" },
                "childAgentId": { "type": "string", "example": "worker-01" },
                "worktreeBranch": { "type": "string", "nullable": true, "example": "swarm/worker-01" },
                "delegatedMask": PermissionMask::openapi_schema(),
                "goal": { "type": "string", "example": "Analyze security audit logs" },
                "createdAt": { "type": "integer", "example": 1731513600 }
            }
        })
    }
}

impl SwarmHandoffEntity {
    #[must_use]
    pub fn to_contract(&self) -> AgentHandoff {
        AgentHandoff {
            handoff_id: self.handoff_id.clone(),
            parent_agent_id: self.parent_agent_id.clone(),
            child_agent_id: self.child_agent_id.clone(),
            worktree_branch: self.worktree_branch.clone(),
            delegated_mask: self.delegated_mask.clone(),
            goal: self.goal.clone(),
        }
    }
}

/// Recorded mediated or signed agent-to-agent message.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MediatedSwarmMessage {
    pub message_id: String,
    pub session_id: String,
    pub from_agent_id: String,
    pub target_agent_id: Option<String>,
    pub payload: Value,
    pub signature: String,
    pub mediated_by_user: Option<String>,
    pub emitted_at_ms: u64,
}

impl OpenApiSchema for MediatedSwarmMessage {
    fn openapi_schema() -> Value {
        serde_json::json!({
            "type": "object",
            "title": "MediatedSwarmMessage",
            "required": ["messageId", "sessionId", "fromAgentId", "payload", "signature", "emittedAtMs"],
            "properties": {
                "messageId": { "type": "string", "example": "msg_01j7xyz" },
                "sessionId": { "type": "string", "example": "ses_01j7sess" },
                "fromAgentId": { "type": "string", "example": "agent-worker" },
                "targetAgentId": { "type": "string", "nullable": true, "example": "agent-leader" },
                "payload": { "type": "object", "example": { "result": "done" } },
                "signature": { "type": "string", "example": "sig_hmac256" },
                "mediatedByUser": { "type": "string", "nullable": true },
                "emittedAtMs": { "type": "integer", "example": 1731513600000_u64 }
            }
        })
    }
}
