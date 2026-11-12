//! Common communication protocol types and event contracts.

use serde::{Deserialize, Serialize};

/// Task request sent to a sandbox runner (Boxer).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskRequest {
    pub task_id: String,
    pub session_id: String,
    pub prompt: String,
    pub timeout_seconds: Option<u64>,
}

/// Task execution progress or stream output.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskStreamEvent {
    pub task_id: String,
    pub chunk: String,
    pub is_stderr: bool,
}

/// Final result status of a task execution.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskResult {
    pub task_id: String,
    pub exit_code: i32,
    pub completed_at: u64,
}
