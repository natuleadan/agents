//! Multi-agent swarm coordination contracts and OWASP ASI-07 permission narrowing.

use std::collections::BTreeSet;
use std::fmt;

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Errors raised when a delegated child agent requests broader permissions than its parent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PermissionError {
    PrivilegeEscalation { capability: &'static str },
    DisallowedTool { tool: String },
}

impl fmt::Display for PermissionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::PrivilegeEscalation { capability } => {
                write!(
                    f,
                    "child agent cannot escalate '{capability}' beyond parent"
                )
            }
            Self::DisallowedTool { tool } => {
                write!(f, "child agent requested tool '{tool}' outside parent mask")
            }
        }
    }
}

impl std::error::Error for PermissionError {}

/// Capability mask enforced across parent-to-child swarm delegations.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PermissionMask {
    pub can_read_fs: bool,
    pub can_write_fs: bool,
    pub can_exec_shell: bool,
    pub can_network: bool,
    #[serde(default)]
    pub allowed_tools: Vec<String>,
}

impl PermissionMask {
    /// Verifies `requested` is strictly a subset of `self`, failing closed on escalation.
    pub fn narrow(&self, requested: &Self) -> Result<Self, PermissionError> {
        if requested.can_read_fs && !self.can_read_fs {
            return Err(PermissionError::PrivilegeEscalation {
                capability: "can_read_fs",
            });
        }
        if requested.can_write_fs && !self.can_write_fs {
            return Err(PermissionError::PrivilegeEscalation {
                capability: "can_write_fs",
            });
        }
        if requested.can_exec_shell && !self.can_exec_shell {
            return Err(PermissionError::PrivilegeEscalation {
                capability: "can_exec_shell",
            });
        }
        if requested.can_network && !self.can_network {
            return Err(PermissionError::PrivilegeEscalation {
                capability: "can_network",
            });
        }

        let parent_tools: BTreeSet<&str> = self.allowed_tools.iter().map(String::as_str).collect();
        for tool in &requested.allowed_tools {
            if !parent_tools.contains(tool.as_str()) {
                return Err(PermissionError::DisallowedTool { tool: tool.clone() });
            }
        }

        Ok(requested.clone())
    }
}

/// Target routing destination on the NATS swarm mesh.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "agentId", rename_all = "camelCase")]
pub enum SwarmTopic {
    Broadcast,
    DirectAgent(String),
}

/// Signed swarm coordination message exchanged via NATS JetStream.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SwarmMessage {
    pub message_id: String,
    pub session_id: String,
    pub from_agent_id: String,
    pub to: SwarmTopic,
    pub payload: Value,
    pub permission_mask: PermissionMask,
    pub signature: String,
    pub emitted_at_ms: u64,
}

/// Bounded subagent delegation descriptor with optional isolated git worktree branch.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentHandoff {
    pub handoff_id: String,
    pub parent_agent_id: String,
    pub child_agent_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub worktree_branch: Option<String>,
    pub delegated_mask: PermissionMask,
    pub goal: String,
}
