//! Validated DTOs for task submission, fencing leases, steering, and completion.

use agents_protocol::{
    ExecOptions, FencingToken, InteractionMode, SandboxProfile, StopReason, TaskState,
};
use garde::Validate;
use ironic::OpenApiSchema;
use serde::Deserialize;

/// Request payload to enqueue a task in a session.
#[derive(Debug, Clone, Deserialize, Validate, OpenApiSchema)]
#[serde(rename_all = "camelCase")]
pub struct SubmitTaskDto {
    #[garde(length(min = 1, max = 32_768))]
    pub prompt: String,
    #[garde(range(min = 5, max = 86_400))]
    pub timeout_seconds: Option<u64>,
    #[garde(skip)]
    pub mode: Option<InteractionMode>,
    #[garde(skip)]
    pub sandbox: Option<SandboxProfile>,
    #[garde(skip)]
    pub exec: Option<ExecOptions>,
}

/// Request from a Boxer worker node to acquire a fenced task lease.
#[derive(Debug, Clone, Deserialize, Validate, OpenApiSchema)]
#[serde(rename_all = "camelCase")]
pub struct LeaseTaskDto {
    #[garde(length(min = 2, max = 128))]
    pub boxer_node_id: String,
    #[garde(range(min = 5, max = 3600))]
    pub lease_ttl_seconds: Option<u64>,
}

/// Fenced intermediate state transition (`Running` or `Blocked`).
#[derive(Debug, Clone, Deserialize, Validate, OpenApiSchema)]
#[serde(rename_all = "camelCase")]
pub struct UpdateTaskStateDto {
    #[garde(skip)]
    pub fencing_token: FencingToken,
    #[garde(skip)]
    pub state: TaskState,
    #[garde(length(max = 256))]
    pub reason: Option<String>,
}

/// Fenced task completion payload from the leased Boxer worker.
#[derive(Debug, Clone, Deserialize, Validate, OpenApiSchema)]
#[serde(rename_all = "camelCase")]
pub struct CompleteTaskDto {
    #[garde(skip)]
    pub fencing_token: FencingToken,
    #[garde(skip)]
    pub exit_code: i32,
    #[garde(skip)]
    pub stop_reason: StopReason,
    #[garde(length(max = 16_384))]
    pub output_summary: Option<String>,
}

/// Live steering injection routed to an active task loop.
#[derive(Debug, Clone, Deserialize, Validate, OpenApiSchema)]
#[serde(rename_all = "camelCase")]
pub struct SteerTaskDto {
    #[garde(length(min = 1, max = 8_192))]
    pub content: String,
}

/// Follow-up prompt queued for the outer loop.
#[derive(Debug, Clone, Deserialize, Validate, OpenApiSchema)]
#[serde(rename_all = "camelCase")]
pub struct FollowUpDto {
    #[garde(length(min = 1, max = 16_384))]
    pub content: String,
}

/// Query parameters for cursor-paginated task listings.
#[derive(Debug, Clone, Default, Deserialize, Validate, OpenApiSchema)]
#[serde(rename_all = "camelCase")]
pub struct ListTasksQuery {
    #[garde(length(max = 128))]
    pub cursor: Option<String>,
    #[garde(range(min = 1, max = 100))]
    pub limit: Option<u32>,
}
