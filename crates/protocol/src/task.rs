//! Task lifecycle, fencing leases, and two-level agent loop controls.

use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::sandbox::{ExecOptions, SandboxProfile};

/// Monotonic fencing token preventing stale Boxer nodes from writing after lease expiry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct FencingToken(pub u64);

/// Durable task execution state tracked in PostgreSQL and NATS.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TaskState {
    Queued,
    Leased,
    Running,
    Blocked,
    Done,
    Failed,
    Cancelled,
    Unknown(String),
}

impl TaskState {
    pub fn as_str(&self) -> &str {
        match self {
            Self::Queued => "queued",
            Self::Leased => "leased",
            Self::Running => "running",
            Self::Blocked => "blocked",
            Self::Done => "done",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
            Self::Unknown(raw) => raw.as_str(),
        }
    }
}

impl Serialize for TaskState {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for TaskState {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let raw = String::deserialize(deserializer)?;
        Ok(match raw.as_str() {
            "queued" => Self::Queued,
            "leased" => Self::Leased,
            "running" => Self::Running,
            "blocked" => Self::Blocked,
            "done" => Self::Done,
            "failed" => Self::Failed,
            "cancelled" => Self::Cancelled,
            _ => Self::Unknown(raw),
        })
    }
}

/// Live Boxer worker status displayed in client worker views.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WorkerStatus {
    Working,
    Blocked,
    Done,
    Idle,
    Unknown(String),
}

impl WorkerStatus {
    pub fn as_str(&self) -> &str {
        match self {
            Self::Working => "working",
            Self::Blocked => "blocked",
            Self::Done => "done",
            Self::Idle => "idle",
            Self::Unknown(raw) => raw.as_str(),
        }
    }
}

impl Serialize for WorkerStatus {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for WorkerStatus {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let raw = String::deserialize(deserializer)?;
        Ok(match raw.as_str() {
            "working" => Self::Working,
            "blocked" => Self::Blocked,
            "done" => Self::Done,
            "idle" => Self::Idle,
            _ => Self::Unknown(raw),
        })
    }
}

/// Agent interaction mode separating read-only planning from state-mutating actions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum InteractionMode {
    Plan,
    #[default]
    Act,
}

/// Outer-loop decision determining whether the native agent runtime continues or settles.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum TurnDecision {
    Continue,
    End,
}

/// Assistant turn termination reason preserved in session history.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StopReason {
    EndTurn,
    ToolUse,
    Length,
    Aborted,
    Error,
    Unknown(String),
}

impl StopReason {
    pub fn as_str(&self) -> &str {
        match self {
            Self::EndTurn => "end_turn",
            Self::ToolUse => "tool_use",
            Self::Length => "length",
            Self::Aborted => "aborted",
            Self::Error => "error",
            Self::Unknown(raw) => raw.as_str(),
        }
    }
}

impl Serialize for StopReason {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for StopReason {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let raw = String::deserialize(deserializer)?;
        Ok(match raw.as_str() {
            "end_turn" => Self::EndTurn,
            "tool_use" => Self::ToolUse,
            "length" => Self::Length,
            "aborted" => Self::Aborted,
            "error" => Self::Error,
            _ => Self::Unknown(raw),
        })
    }
}

/// Task request dispatched from Server to Boxer via NATS JetStream.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskRequest {
    pub task_id: String,
    pub session_id: String,
    pub prompt: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout_seconds: Option<u64>,
    #[serde(default)]
    pub mode: InteractionMode,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fencing_token: Option<FencingToken>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub idempotency_key: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sandbox: Option<SandboxProfile>,
    #[serde(default)]
    pub exec: ExecOptions,
}

/// Active lease granted to a Boxer worker for executing a task.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskLease {
    pub task_id: String,
    pub session_id: String,
    pub boxer_node_id: String,
    pub fencing_token: FencingToken,
    pub acquired_at_ms: u64,
    pub expires_at_ms: u64,
}

/// Live steering injection routed into the inner tool loop during execution.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SteeringMessage {
    pub message_id: String,
    pub session_id: String,
    pub task_id: String,
    pub content: String,
    pub emitted_at_ms: u64,
}

/// Queued follow-up prompt evaluated by the outer loop after the active turn ends.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FollowUpMessage {
    pub message_id: String,
    pub session_id: String,
    pub content: String,
    pub queued_at_ms: u64,
}

/// Final completion summary returned when a task settles.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskResult {
    pub task_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fencing_token: Option<FencingToken>,
    pub exit_code: i32,
    pub stop_reason: StopReason,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_summary: Option<String>,
    pub completed_at: u64,
}
