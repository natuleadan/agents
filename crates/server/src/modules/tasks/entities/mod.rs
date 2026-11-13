//! Durable fenced task entity and strict state-machine transitions.

use agents_protocol::{FencingToken, InteractionMode, StopReason, TaskState};
use serde::{Deserialize, Serialize};

/// Persisted execution task coordinated between Server and Boxer via NATS & PostgreSQL.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskEntity {
    pub id: String,
    pub session_id: String,
    pub owner_id: String,
    pub prompt: String,
    pub state: TaskState,
    pub mode: InteractionMode,
    pub fencing_token: FencingToken,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lease_holder: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lease_expires_at: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub idempotency_key: Option<String>,
    pub version: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exit_code: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stop_reason: Option<StopReason>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_summary: Option<String>,
    pub created_at: u64,
    pub updated_at: u64,
}

impl TaskEntity {
    #[must_use]
    pub fn is_terminal(&self) -> bool {
        matches!(
            self.state,
            TaskState::Done | TaskState::Failed | TaskState::Cancelled
        )
    }
}

/// Validates durable state transitions (`Queued -> Leased -> Running <-> Blocked -> Done|Failed|Cancelled`).
#[must_use]
pub const fn is_valid_transition(from: &TaskState, to: &TaskState) -> bool {
    matches!(
        (from, to),
        (TaskState::Queued, TaskState::Leased | TaskState::Cancelled)
            | (
                TaskState::Leased,
                TaskState::Running | TaskState::Failed | TaskState::Cancelled,
            )
            | (
                TaskState::Running,
                TaskState::Blocked | TaskState::Done | TaskState::Failed | TaskState::Cancelled,
            )
            | (
                TaskState::Blocked,
                TaskState::Running | TaskState::Done | TaskState::Failed | TaskState::Cancelled,
            )
    )
}
