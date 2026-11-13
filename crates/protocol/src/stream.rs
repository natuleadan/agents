//! Ordered stream events and reconnect reconciliation envelopes.

use serde::{Deserialize, Serialize};

use crate::preview::TraefikPreviewRoute;
use crate::task::{StopReason, TurnDecision, WorkerStatus};
use crate::version::CodecId;

/// Legacy-compatible chunk stream output enriched with sequence ordering.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskStreamEvent {
    pub task_id: String,
    #[serde(default)]
    pub seq: u64,
    #[serde(default)]
    pub emitted_at_ms: u64,
    pub chunk: String,
    pub is_stderr: bool,
}

/// Lifecycle phase of an individual tool invocation inside the agent loop.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ToolPhase {
    Started,
    Completed,
    Failed,
}

/// Discriminated stream payload with forward-compatible `Unknown` fallback.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum StreamPayload {
    TaskChunk {
        chunk: String,
        is_stderr: bool,
    },
    WorkerStateChanged {
        node_id: String,
        status: WorkerStatus,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        reason: Option<String>,
    },
    AgentTurnLifecycle {
        turn_id: String,
        decision: TurnDecision,
        stop_reason: StopReason,
    },
    SteeringAck {
        message_id: String,
        accepted: bool,
    },
    ToolCallEvent {
        call_id: String,
        tool_name: String,
        phase: ToolPhase,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        error: Option<String>,
    },
    PreviewPublished(TraefikPreviewRoute),
    AutocronTriggered {
        schedule_id: String,
        workflow_id: String,
        triggered_at_ms: u64,
    },
    #[serde(other)]
    Unknown,
}

/// Canonical ordered envelope carried across NATS JetStream, WebSockets, and SSE.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EventEnvelope {
    pub seq: u64,
    pub emitted_at_ms: u64,
    pub codec: CodecId,
    pub session_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task_id: Option<String>,
    pub payload: StreamPayload,
}
