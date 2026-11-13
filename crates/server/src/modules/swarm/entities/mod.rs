//! Swarm domain entities representing delegated subagents and mediated messages.

use agents_protocol::{AgentHandoff, PermissionMask};
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
