//! DTOs and typed RFC 7807 problem details for the Boxer HTTP control plane API.

use std::collections::HashMap;

use agents_protocol::SandboxProfile;
use serde::{Deserialize, Serialize};

/// Request to provision a new sandbox container.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProvisionBoxRequest {
    pub session_id: String,
    pub task_id: String,
    #[serde(default)]
    pub profile: Option<SandboxProfile>,
    #[serde(default)]
    pub env: HashMap<String, String>,
}

/// Acknowledgment returned upon successful sandbox provisioning.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ProvisionBoxResponse {
    pub box_id: String,
    pub status: String,
    pub working_dir: String,
    pub created_at: u64,
}

/// Request to execute an arbitrary command inside an existing sandbox.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExecBoxRequest {
    pub command: String,
    #[serde(default)]
    pub timeout_ms: Option<u64>,
}

/// Result of executing a command inside a sandbox.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ExecBoxResponse {
    pub exit_code: i32,
    pub stdout: String,
    pub stderr: String,
    pub timed_out: bool,
    pub cancelled: bool,
    pub duration_ms: u64,
}

/// Telemetry and lifecycle metrics for an active sandbox.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct BoxStatusResponse {
    pub box_id: String,
    pub status: String,
    pub memory_bytes: u64,
    pub active_tasks: u32,
    pub uptime_secs: u64,
}

/// Request to write a file into the sandbox working directory.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WriteFileRequest {
    pub path: String,
    pub content: String,
}

/// Content of a file read from the sandbox working directory.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ReadFileResponse {
    pub path: String,
    pub content: String,
    pub size: u64,
}

/// Confirmation of sandbox teardown.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct TeardownBoxResponse {
    pub box_id: String,
    pub status: String,
}

/// Node-level health and capacity report.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct BoxerHealthResponse {
    pub status: String,
    pub node_id: String,
    pub active_boxes: usize,
    pub max_concurrent_boxes: usize,
}

/// RFC 7807 compliant problem details error envelope.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BoxerProblemDetails {
    #[serde(rename = "type")]
    pub error_type: String,
    pub title: String,
    pub status: u16,
    pub detail: String,
}
