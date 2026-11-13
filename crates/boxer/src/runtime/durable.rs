//! Pi-Durable agent step execution logger and crash-resume replay tracker.
//!
//! Enforces zero duplicate side-effects on restart by persisting append-only step records
//! and replaying previously executed tool invocations and state checkpoints.

use std::{collections::HashMap, sync::Arc};

use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::sync::Mutex;

use crate::{config::now_ms, error::BoxerError, leases::BoxFuture};

/// Distinct event kinds captured in the durable step log.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DurableStepKind {
    ToolCall,
    TurnDecision,
    OutputChunk,
    Steering,
    Checkpoint,
}

impl DurableStepKind {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ToolCall => "tool_call",
            Self::TurnDecision => "turn_decision",
            Self::OutputChunk => "output_chunk",
            Self::Steering => "steering",
            Self::Checkpoint => "checkpoint",
        }
    }
}

/// Append-only record capturing a single discrete execution step.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DurableStepRecord {
    pub step_id: String,
    pub task_id: String,
    pub session_id: String,
    pub step_index: u32,
    pub step_kind: DurableStepKind,
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

/// Abstract contract for append-only durable step logging.
pub trait DurableStepLogger: Send + Sync + 'static {
    fn append_step<'a>(
        &'a self,
        record: DurableStepRecord,
    ) -> BoxFuture<'a, Result<(), BoxerError>>;

    fn list_steps<'a>(
        &'a self,
        task_id: &'a str,
    ) -> BoxFuture<'a, Result<Vec<DurableStepRecord>, BoxerError>>;
}

/// Thread-safe in-memory implementation of [`DurableStepLogger`].
#[derive(Clone, Default)]
pub struct InMemoryDurableLogger {
    steps: Arc<Mutex<HashMap<String, Vec<DurableStepRecord>>>>,
}

impl InMemoryDurableLogger {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }
}

impl DurableStepLogger for InMemoryDurableLogger {
    fn append_step<'a>(
        &'a self,
        record: DurableStepRecord,
    ) -> BoxFuture<'a, Result<(), BoxerError>> {
        Box::pin(async move {
            let mut lock = self.steps.lock().await;
            lock.entry(record.task_id.clone()).or_default().push(record);
            Ok(())
        })
    }

    fn list_steps<'a>(
        &'a self,
        task_id: &'a str,
    ) -> BoxFuture<'a, Result<Vec<DurableStepRecord>, BoxerError>> {
        Box::pin(async move {
            let lock = self.steps.lock().await;
            Ok(lock.get(task_id).cloned().unwrap_or_default())
        })
    }
}

/// Replay tracker maintaining completed side effects to ensure idempotency upon crash recovery.
#[derive(Debug, Clone, Default)]
pub struct DurableReplayTracker {
    task_id: String,
    session_id: String,
    next_step_index: u32,
    completed_tools: HashMap<String, DurableStepRecord>,
    last_checkpoint: Option<Value>,
}

impl DurableReplayTracker {
    #[must_use]
    pub fn new(task_id: impl Into<String>, session_id: impl Into<String>) -> Self {
        Self {
            task_id: task_id.into(),
            session_id: session_id.into(),
            next_step_index: 0,
            completed_tools: HashMap::new(),
            last_checkpoint: None,
        }
    }

    /// Initializes tracker from historical log entries, restoring completed tool side-effects.
    pub fn recover_from_steps(&mut self, steps: &[DurableStepRecord]) {
        for step in steps {
            if step.step_index >= self.next_step_index {
                self.next_step_index = step.step_index.saturating_add(1);
            }
            if step.step_kind == DurableStepKind::ToolCall {
                if let Some(call_id) = &step.call_id {
                    self.completed_tools.insert(call_id.clone(), step.clone());
                }
            } else if step.step_kind == DurableStepKind::Checkpoint {
                self.last_checkpoint = step.output_payload.clone();
            }
        }
    }

    /// Queries whether a tool invocation already completed in a prior turn/crash.
    #[must_use]
    pub fn get_cached_tool_result(&self, call_id: &str) -> Option<&DurableStepRecord> {
        self.completed_tools.get(call_id)
    }

    /// Checks if a tool call was previously executed.
    #[must_use]
    pub fn has_tool_completed(&self, call_id: &str) -> bool {
        self.completed_tools.contains_key(call_id)
    }

    /// Returns the last persisted checkpoint state, if any.
    #[must_use]
    pub fn last_checkpoint(&self) -> Option<&Value> {
        self.last_checkpoint.as_ref()
    }

    /// Prepares a new tool call step record for persistence.
    pub fn build_tool_step(
        &mut self,
        call_id: impl Into<String>,
        tool_name: impl Into<String>,
        input: Value,
        output: Value,
        is_error: bool,
    ) -> DurableStepRecord {
        let call_id = call_id.into();
        let idx = self.next_step_index;
        self.next_step_index = self.next_step_index.saturating_add(1);

        let record = DurableStepRecord {
            step_id: uuid::Uuid::new_v4().to_string(),
            task_id: self.task_id.clone(),
            session_id: self.session_id.clone(),
            step_index: idx,
            step_kind: DurableStepKind::ToolCall,
            call_id: Some(call_id.clone()),
            tool_name: Some(tool_name.into()),
            input_payload: Some(input),
            output_payload: Some(output),
            is_error,
            recorded_at_ms: now_ms(),
        };

        self.completed_tools.insert(call_id, record.clone());
        record
    }

    /// Prepares a new checkpoint step record for persistence.
    pub fn build_checkpoint_step(&mut self, state: Value) -> DurableStepRecord {
        let idx = self.next_step_index;
        self.next_step_index = self.next_step_index.saturating_add(1);

        let record = DurableStepRecord {
            step_id: uuid::Uuid::new_v4().to_string(),
            task_id: self.task_id.clone(),
            session_id: self.session_id.clone(),
            step_index: idx,
            step_kind: DurableStepKind::Checkpoint,
            call_id: None,
            tool_name: None,
            input_payload: None,
            output_payload: Some(state.clone()),
            is_error: false,
            recorded_at_ms: now_ms(),
        };

        self.last_checkpoint = Some(state);
        record
    }
}
