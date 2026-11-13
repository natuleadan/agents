//! Native two-level Rust agent runtime executing parallel-safe tools in source order,
//! handling live steering, aborting on `length`, and retrying without polluting context.

use std::{
    collections::HashMap,
    sync::{Arc, atomic::AtomicU64},
};

use agents_protocol::{
    EventEnvelope, FollowUpMessage, InteractionMode, PermissionMask, SteeringMessage, StopReason,
    TurnDecision,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::sync::{mpsc, watch};

use crate::{
    config::now_ms,
    error::BoxerError,
    leases::BoxFuture,
    runtime::{
        hooks::{LoopHooks, OutputSchemaContract, RetryPolicy},
        providers::{ModelClient, ModelTurnRequest, ModelTurnResponse},
        session_tree::{SessionEntry, SessionTree},
    },
};

/// Tool call emitted by a model turn response.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolCallRequest {
    pub call_id: String,
    pub tool_name: String,
    pub input: Value,
}

/// Structured tool execution outcome appended to the session tree.
#[derive(Debug, Clone)]
pub struct ToolExecutionResult {
    pub call_id: String,
    pub tool_name: String,
    pub output: String,
    pub is_error: bool,
}

/// Tool contract executed inside [`NativeAgentRuntime`].
pub trait RuntimeTool: Send + Sync + 'static {
    fn name(&self) -> &str;

    fn is_mutating(&self) -> bool {
        false
    }

    fn parallel_safe(&self) -> bool {
        !self.is_mutating()
    }

    fn execute<'a>(&'a self, input: Value) -> BoxFuture<'a, Result<String, BoxerError>>;
}

/// Audit entry preserving raw failed provider retries omitted from model context.
#[derive(Debug, Clone)]
pub struct RetryAuditEntry {
    pub turn_index: u32,
    pub attempt: u32,
    pub error: String,
    pub omitted_from_context: bool,
    pub recorded_at_ms: u64,
}

/// Runtime parameters controlling compaction, retry backoff, CodeMode, and modes.
#[derive(Debug, Clone)]
pub struct NativeRuntimeConfig {
    pub task_id: String,
    pub endpoint_id: String,
    pub mode: InteractionMode,
    pub permissions: PermissionMask,
    pub max_turns: u32,
    pub compact_keep_tail: usize,
    pub retry_policy: RetryPolicy,
    pub output_contract: Option<OutputSchemaContract>,
}

/// Final outcome produced by [`NativeAgentRuntime::run`].
#[derive(Debug, Clone)]
pub struct NativeTurnOutcome {
    pub turns_executed: u32,
    pub stop_reason: StopReason,
    pub final_text: String,
    pub structured_json: Option<Value>,
    pub steering_acks: u32,
    pub aborted_length_batches: u32,
    pub retry_audit: Vec<RetryAuditEntry>,
}

/// Two-level native Rust agent loop driving providers, parallel tools, steering, and compaction.
pub struct NativeAgentRuntime {
    model: Arc<dyn ModelClient>,
    pub(super) tools: HashMap<String, Arc<dyn RuntimeTool>>,
    hooks: Arc<dyn LoopHooks>,
    pub(super) seq: Arc<AtomicU64>,
}

impl NativeAgentRuntime {
    #[must_use]
    pub fn new(model: Arc<dyn ModelClient>, hooks: Arc<dyn LoopHooks>) -> Self {
        Self {
            model,
            tools: HashMap::new(),
            hooks,
            seq: Arc::new(AtomicU64::new(1)),
        }
    }

    pub fn register_tool<T: RuntimeTool>(&mut self, tool: T) {
        self.tools.insert(tool.name().to_owned(), Arc::new(tool));
    }

    /// Executes inner tool loops and outer continuation turns over `tree`.
    ///
    /// # Errors
    ///
    /// Returns [`BoxerError`] only on structural CodeMode output contract failure.
    pub async fn run(
        &self,
        cfg: &NativeRuntimeConfig,
        tree: &mut SessionTree,
        events_tx: mpsc::Sender<EventEnvelope>,
        mut steering_rx: mpsc::Receiver<SteeringMessage>,
        mut follow_up_rx: mpsc::Receiver<FollowUpMessage>,
        cancel_rx: watch::Receiver<bool>,
    ) -> Result<NativeTurnOutcome, BoxerError> {
        let mut turns_executed: u32 = 0;
        let mut steering_acks: u32 = 0;
        let mut aborted_length_batches: u32 = 0;
        let mut retry_audit = Vec::new();
        let mut final_text = String::new();
        let stop_reason: StopReason;
        let mut current_mode = cfg.mode;

        loop {
            if *cancel_rx.borrow() {
                stop_reason = StopReason::Aborted;
                self.emit_turn(
                    &events_tx,
                    &tree.session_id,
                    &cfg.task_id,
                    turns_executed,
                    TurnDecision::End,
                    StopReason::Aborted,
                )
                .await;
                break;
            }

            turns_executed = turns_executed.saturating_add(1);
            if turns_executed > cfg.max_turns.max(1) {
                stop_reason = StopReason::Length;
                self.emit_turn(
                    &events_tx,
                    &tree.session_id,
                    &cfg.task_id,
                    turns_executed,
                    TurnDecision::End,
                    StopReason::Length,
                )
                .await;
                break;
            }

            let compacted = tree.compact_for_request(cfg.compact_keep_tail);
            let raw_req = ModelTurnRequest {
                session_id: tree.session_id.clone(),
                turn_index: turns_executed,
                endpoint_id: cfg.endpoint_id.clone(),
                entries: compacted,
                available_tools: self.tools.keys().cloned().collect(),
                json_schema_mode: cfg.output_contract.is_some(),
            };
            let prepared_req = self
                .hooks
                .prepare_request(tree, raw_req, current_mode)
                .await?;

            let resp = match self
                .call_with_retry(
                    turns_executed,
                    prepared_req,
                    &cfg.retry_policy,
                    &cancel_rx,
                    &mut retry_audit,
                )
                .await
            {
                Ok(r) => r,
                Err(BoxerError::Cancelled(_)) => {
                    stop_reason = StopReason::Aborted;
                    self.emit_turn(
                        &events_tx,
                        &tree.session_id,
                        &cfg.task_id,
                        turns_executed,
                        TurnDecision::End,
                        StopReason::Aborted,
                    )
                    .await;
                    break;
                }
                Err(_) => {
                    stop_reason = StopReason::Error;
                    self.emit_turn(
                        &events_tx,
                        &tree.session_id,
                        &cfg.task_id,
                        turns_executed,
                        TurnDecision::End,
                        StopReason::Error,
                    )
                    .await;
                    break;
                }
            };

            self.hooks.finish_turn(turns_executed, &resp).await?;

            if !resp.assistant_text.is_empty() {
                final_text = resp.assistant_text.clone();
                tree.append(SessionEntry::AssistantText {
                    text: resp.assistant_text.clone(),
                });
            }

            // Abort the entire tool batch cleanly when model terminates due to `length`.
            if resp.stop_reason == StopReason::Length {
                aborted_length_batches = aborted_length_batches.saturating_add(1);
                stop_reason = StopReason::Length;
                self.emit_turn(
                    &events_tx,
                    &tree.session_id,
                    &cfg.task_id,
                    turns_executed,
                    TurnDecision::End,
                    StopReason::Length,
                )
                .await;
                break;
            }

            if !resp.tool_calls.is_empty() {
                let (steered, was_cancelled) = self
                    .execute_tool_batch(
                        cfg,
                        &mut current_mode,
                        tree,
                        &resp.tool_calls,
                        &events_tx,
                        &mut steering_rx,
                        &cancel_rx,
                    )
                    .await;
                steering_acks = steering_acks.saturating_add(steered);
                if was_cancelled {
                    stop_reason = StopReason::Aborted;
                    self.emit_turn(
                        &events_tx,
                        &tree.session_id,
                        &cfg.task_id,
                        turns_executed,
                        TurnDecision::End,
                        StopReason::Aborted,
                    )
                    .await;
                    break;
                }
                // Tool results appended -> outer loop continues turn deterministically.
                self.emit_turn(
                    &events_tx,
                    &tree.session_id,
                    &cfg.task_id,
                    turns_executed,
                    TurnDecision::Continue,
                    StopReason::ToolUse,
                )
                .await;
                continue;
            }

            let pending_follow_up = follow_up_rx.try_recv().ok();
            let decision = self
                .hooks
                .prepare_next_turn(turns_executed, tree, pending_follow_up.as_ref())
                .await;

            if let Some(fu) = pending_follow_up {
                tree.append(SessionEntry::UserMessage { text: fu.content });
            }

            if decision == TurnDecision::Continue {
                self.emit_turn(
                    &events_tx,
                    &tree.session_id,
                    &cfg.task_id,
                    turns_executed,
                    TurnDecision::Continue,
                    resp.stop_reason,
                )
                .await;
                continue;
            }

            stop_reason = resp.stop_reason;
            self.emit_turn(
                &events_tx,
                &tree.session_id,
                &cfg.task_id,
                turns_executed,
                TurnDecision::End,
                stop_reason.clone(),
            )
            .await;
            break;
        }

        let structured_json = if stop_reason == StopReason::EndTurn
            && let Some(contract) = &cfg.output_contract
        {
            Some(contract.validate(&final_text)?)
        } else {
            None
        };

        Ok(NativeTurnOutcome {
            turns_executed,
            stop_reason,
            final_text,
            structured_json,
            steering_acks,
            aborted_length_batches,
            retry_audit,
        })
    }

    async fn call_with_retry(
        &self,
        turn_index: u32,
        req: ModelTurnRequest,
        policy: &RetryPolicy,
        cancel_rx: &watch::Receiver<bool>,
        audit: &mut Vec<RetryAuditEntry>,
    ) -> Result<ModelTurnResponse, BoxerError> {
        let max_attempts = policy.max_retries.saturating_add(1);
        for attempt in 0..max_attempts {
            if *cancel_rx.borrow() {
                return Err(BoxerError::Cancelled(
                    "aborted before model inference".into(),
                ));
            }
            match self.model.complete_turn(req.clone()).await {
                Ok(resp) => return Ok(resp),
                Err(err) => {
                    audit.push(RetryAuditEntry {
                        turn_index,
                        attempt,
                        error: err.to_string(),
                        omitted_from_context: true,
                        recorded_at_ms: now_ms(),
                    });
                    if attempt + 1 >= max_attempts {
                        return Err(err);
                    }
                    tokio::time::sleep(policy.backoff_for_attempt(attempt)).await;
                }
            }
        }
        Err(BoxerError::ExecutionFailed("retry loop exhausted".into()))
    }
}
