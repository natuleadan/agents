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
    #[garde(skip)]
    pub webhook_slug: Option<String>,
    #[garde(skip)]
    pub render_slug: Option<String>,
    #[garde(skip)]
    pub created_by_agent: Option<bool>,
}

/// Request payload to approve an agent-created or pending workflow.
#[derive(Debug, Clone, Default, Deserialize, Validate)]
#[serde(rename_all = "camelCase")]
pub struct ApproveWorkflowDto {
    #[garde(skip)]
    pub notes: Option<String>,
}

/// Request payload to publish rendered HTML or static content to a workflow render slug.
#[derive(Debug, Clone, Deserialize, Validate)]
#[serde(rename_all = "camelCase")]
pub struct PublishRenderSlugDto {
    #[garde(length(min = 1))]
    pub content: String,
    #[garde(skip)]
    pub content_type: Option<String>,
}

/// Request payload to manually trigger a workflow with optional initial context input.
#[derive(Debug, Clone, Default, Deserialize, Validate)]
#[serde(rename_all = "camelCase")]
pub struct TriggerWorkflowDto {
    #[garde(skip)]
    pub input: Option<Value>,
}

/// Request payload accepted by incoming dynamic webhook slugs.
#[derive(Debug, Clone, Default, Deserialize, Validate)]
#[serde(rename_all = "camelCase")]
pub struct WebhookPayloadDto {
    #[garde(skip)]
    pub payload: Option<Value>,
}
