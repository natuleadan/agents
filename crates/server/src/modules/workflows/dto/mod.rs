//! Validated DTOs for creating and executing DAG workflows.

use agents_protocol::{WorkflowEdge, WorkflowNode};
use garde::Validate;
use serde::Deserialize;
use serde_json::Value;

/// Request payload to create an automated DAG workflow.
#[derive(Debug, Clone, Deserialize, Validate)]
#[serde(rename_all = "camelCase")]
pub struct CreateWorkflowDto {
    #[garde(length(min = 1, max = 160))]
    pub name: String,
    #[garde(length(min = 1))]
    pub nodes: Vec<WorkflowNode>,
    #[garde(skip)]
    pub edges: Vec<WorkflowEdge>,
}

/// Request payload to manually trigger a workflow with optional initial context input.
#[derive(Debug, Clone, Default, Deserialize, Validate)]
#[serde(rename_all = "camelCase")]
pub struct TriggerWorkflowDto {
    #[garde(skip)]
    pub input: Option<Value>,
}
