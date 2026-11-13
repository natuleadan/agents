//! Domain entities for node-based DAG workflows and crash-resumable workflow runs.

use agents_protocol::{WorkflowDag, WorkflowEdge, WorkflowNode};
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Persisted DAG workflow definition owned by a user.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowEntity {
    pub workflow_id: String,
    pub owner_id: String,
    pub name: String,
    pub nodes: Vec<WorkflowNode>,
    pub edges: Vec<WorkflowEdge>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub webhook_slug: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub render_slug: Option<String>,
    #[serde(default)]
    pub approved: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub approved_by: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub approved_at: Option<u64>,
    #[serde(default)]
    pub created_by_agent: bool,
    pub created_at: u64,
    pub updated_at: u64,
}

impl WorkflowEntity {
    #[must_use]
    pub fn to_dag(&self) -> WorkflowDag {
        WorkflowDag {
            workflow_id: self.workflow_id.clone(),
            name: self.name.clone(),
            nodes: self.nodes.clone(),
            edges: self.edges.clone(),
            webhook_slug: self.webhook_slug.clone(),
            render_slug: self.render_slug.clone(),
            approved: Some(self.approved),
            created_by_agent: Some(self.created_by_agent),
        }
    }
}

/// Persistent execution state of a workflow run supporting `boot_id` crash recovery.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum WorkflowRunStatus {
    Running,
    Completed,
    Failed,
    Interrupted,
}

impl WorkflowRunStatus {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Running => "running",
            Self::Completed => "completed",
            Self::Failed => "failed",
            Self::Interrupted => "interrupted",
        }
    }

    #[must_use]
    pub fn parse(raw: &str) -> Self {
        match raw {
            "completed" => Self::Completed,
            "failed" => Self::Failed,
            "interrupted" => Self::Interrupted,
            _ => Self::Running,
        }
    }
}

/// Checkpoint-persisted execution instance of a DAG workflow.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowRunEntity {
    pub run_id: String,
    pub workflow_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub schedule_id: Option<String>,
    pub owner_id: String,
    pub boot_id: String,
    pub status: WorkflowRunStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub current_node_id: Option<String>,
    pub checkpoint_state: Value,
    pub resumed_count: u32,
    pub started_at: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub completed_at: Option<u64>,
}

/// Rendered HTML or dynamic payload served at `/render/:slug`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RenderedSlugEntity {
    pub slug: String,
    pub workflow_id: String,
    pub content_type: String,
    pub content: String,
    pub approved: bool,
    pub updated_at: u64,
}

/// Recorded step inside durable agent step logger for crash replay.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DurableStepEntity {
    pub step_id: String,
    pub task_id: String,
    pub session_id: String,
    pub step_index: u32,
    pub step_kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub call_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input_payload: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_payload: Option<Value>,
    pub is_error: bool,
    pub recorded_at_ms: u64,
}
