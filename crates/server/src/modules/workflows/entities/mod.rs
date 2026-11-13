//! Domain entities for node-based DAG workflows and crash-resumable workflow runs.

use agents_protocol::{WorkflowDag, WorkflowEdge, WorkflowNode};
use ironic::OpenApiSchema;
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

impl OpenApiSchema for WorkflowEntity {
    fn openapi_schema() -> Value {
        serde_json::json!({
            "type": "object",
            "title": "WorkflowEntity",
            "required": ["workflowId", "ownerId", "name", "nodes", "edges", "approved", "createdByAgent", "createdAt", "updatedAt"],
            "properties": {
                "workflowId": { "type": "string", "example": "wf_01j7dag001" },
                "ownerId": { "type": "string", "example": "usr_01j7abc" },
                "name": { "type": "string", "example": "Analysis & Publishing Pipeline" },
                "nodes": { "type": "array", "items": WorkflowNode::openapi_schema() },
                "edges": { "type": "array", "items": WorkflowEdge::openapi_schema() },
                "webhookSlug": { "type": "string", "nullable": true, "example": "ingress-analysis" },
                "renderSlug": { "type": "string", "nullable": true, "example": "view-analysis" },
                "approved": { "type": "boolean", "example": true },
                "approvedBy": { "type": "string", "nullable": true, "example": "admin-supervisor" },
                "approvedAt": { "type": "integer", "nullable": true, "example": 1731513600 },
                "createdByAgent": { "type": "boolean", "example": false },
                "createdAt": { "type": "integer", "example": 1731513000 },
                "updatedAt": { "type": "integer", "example": 1731513600 }
            }
        })
    }
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

impl OpenApiSchema for WorkflowRunStatus {
    fn openapi_schema() -> Value {
        serde_json::json!({
            "type": "string",
            "title": "WorkflowRunStatus",
            "enum": ["running", "completed", "failed", "interrupted"],
            "example": "completed"
        })
    }
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

impl OpenApiSchema for WorkflowRunEntity {
    fn openapi_schema() -> Value {
        serde_json::json!({
            "type": "object",
            "title": "WorkflowRunEntity",
            "required": ["runId", "workflowId", "ownerId", "bootId", "status", "checkpointState", "resumedCount", "startedAt"],
            "properties": {
                "runId": { "type": "string", "example": "run_01j7xyz" },
                "workflowId": { "type": "string", "example": "wf_01j7dag001" },
                "scheduleId": { "type": "string", "nullable": true },
                "ownerId": { "type": "string", "example": "usr_01j7abc" },
                "bootId": { "type": "string", "example": "boot_123" },
                "status": WorkflowRunStatus::openapi_schema(),
                "currentNodeId": { "type": "string", "nullable": true, "example": "node_1" },
                "checkpointState": { "type": "object", "example": { "step": 1 } },
                "resumedCount": { "type": "integer", "example": 0 },
                "startedAt": { "type": "integer", "example": 1731513600 },
                "completedAt": { "type": "integer", "nullable": true, "example": 1731513605 }
            }
        })
    }
}

/// Rendered HTML or dynamic payload served at `/render/:slug`.
#[derive(Debug, Clone, Serialize, Deserialize, OpenApiSchema)]
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
