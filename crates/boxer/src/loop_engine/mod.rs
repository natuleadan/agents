//! Native dual-loop agent execution engine (inner tool loop with live steering
//! and outer follow-up / `TurnDecision` settlement loop).

use std::{
    collections::HashMap,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
};

use agents_protocol::{
    CodecId, EventEnvelope, FollowUpMessage, InteractionMode, PermissionMask, SteeringMessage,
    StopReason, StreamPayload, ToolPhase, TurnDecision,
};
use serde_json::Value;
use tokio::sync::{mpsc, watch};

use crate::{config::now_ms, error::BoxerError, leases::BoxFuture};

/// Single tool invocation planned for an inner-loop turn.
#[derive(Debug, Clone)]
pub struct ToolInvocation {
    pub call_id: String,
    pub tool_name: String,
    pub input: Value,
}

/// Abstract tool executed inside the Boxer inner loop.
pub trait BoxerTool: Send + Sync + 'static {
    fn name(&self) -> &str;

    fn is_mutating(&self) -> bool {
        false
    }

    fn requires_network(&self) -> bool {
        false
    }

    fn execute<'a>(&'a self, input: Value) -> BoxFuture<'a, Result<String, BoxerError>>;
}

/// Registry of tools available to a dual-loop agent session.
#[derive(Default)]
pub struct ToolRegistry {
    tools: HashMap<String, Arc<dyn BoxerTool>>,
}

impl ToolRegistry {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register<T: BoxerTool>(&mut self, tool: T) {
        self.tools.insert(tool.name().to_owned(), Arc::new(tool));
    }

    #[must_use]
    pub fn get(&self, name: &str) -> Option<&Arc<dyn BoxerTool>> {
        self.tools.get(name)
    }
}

/// Planner deciding tool batches for initial prompt, steering redirects, and outer follow-ups.
pub trait TurnPlanner: Send + Sync + 'static {
    fn plan_initial(&self, prompt: &str) -> Vec<ToolInvocation>;

    fn on_steering(&self, steering: &SteeringMessage) -> Vec<ToolInvocation> {
        let _ = steering;
        Vec::new()
    }

    fn on_follow_up(&self, follow_up: &FollowUpMessage) -> Vec<ToolInvocation> {
        let _ = follow_up;
        Vec::new()
    }
}

/// Summary produced after inner and outer loops finish.
#[derive(Debug, Clone)]
pub struct LoopRunReport {
    pub turns_executed: u32,
    pub tool_calls_completed: u32,
    pub tool_calls_failed: u32,
    pub steering_applied: u32,
    pub follow_ups_applied: u32,
    pub stop_reason: StopReason,
    pub final_output: String,
}

/// Context passed into [`DualLoopEngine::run`].
pub struct LoopRunContext {
    pub session_id: String,
    pub task_id: String,
    pub initial_prompt: String,
    pub mode: InteractionMode,
    pub permissions: PermissionMask,
    pub max_turns: u32,
}

/// Executes inner tool-call turns and outer follow-up continuation loops.
pub struct DualLoopEngine {
    registry: Arc<ToolRegistry>,
    planner: Arc<dyn TurnPlanner>,
    seq: Arc<AtomicU64>,
}

impl DualLoopEngine {
    #[must_use]
    pub fn new(registry: Arc<ToolRegistry>, planner: Arc<dyn TurnPlanner>) -> Self {
        Self {
            registry,
            planner,
            seq: Arc::new(AtomicU64::new(1)),
        }
    }

    /// Runs the dual-level agent loop until settlement, follow-up drain, or cancellation.
    ///
    /// # Errors
    ///
    /// Returns [`BoxerError`] on terminal bus/execution errors.
    pub async fn run(
        &self,
        ctx: LoopRunContext,
        events_tx: mpsc::Sender<EventEnvelope>,
        mut steering_rx: mpsc::Receiver<SteeringMessage>,
        mut follow_up_rx: mpsc::Receiver<FollowUpMessage>,
        cancel_rx: watch::Receiver<bool>,
    ) -> Result<LoopRunReport, BoxerError> {
        let mut pending_queue = self.planner.plan_initial(&ctx.initial_prompt);
        let mut turns_executed: u32 = 0;
        let mut tool_calls_completed: u32 = 0;
        let mut tool_calls_failed: u32 = 0;
        let mut steering_applied: u32 = 0;
        let mut follow_ups_applied: u32 = 0;
        let mut outputs = Vec::new();
        let final_stop: StopReason;

        loop {
            if *cancel_rx.borrow() {
                final_stop = StopReason::Aborted;
                self.emit_turn_lifecycle(
                    &events_tx,
                    &ctx,
                    turns_executed,
                    TurnDecision::End,
                    StopReason::Aborted,
                )
                .await;
                break;
            }

            turns_executed = turns_executed.saturating_add(1);
            if turns_executed > ctx.max_turns.max(1) {
                final_stop = StopReason::Length;
                self.emit_turn_lifecycle(
                    &events_tx,
                    &ctx,
                    turns_executed,
                    TurnDecision::End,
                    StopReason::Length,
                )
                .await;
                break;
            }

            let mut idx = 0;
            let mut turn_aborted = false;
            while idx < pending_queue.len() {
                if *cancel_rx.borrow() {
                    turn_aborted = true;
                    break;
                }

                // Drain live steering between inner steps and redirect remaining steps cleanly.
                while let Ok(steer) = steering_rx.try_recv() {
                    steering_applied = steering_applied.saturating_add(1);
                    self.emit(
                        &events_tx,
                        &ctx.session_id,
                        Some(&ctx.task_id),
                        StreamPayload::SteeringAck {
                            message_id: steer.message_id.clone(),
                            accepted: true,
                        },
                    )
                    .await;
                    let redirected = self.planner.on_steering(&steer);
                    if !redirected.is_empty() {
                        pending_queue.truncate(idx);
                        pending_queue.extend(redirected);
                    }
                }

                if idx >= pending_queue.len() {
                    break;
                }
                let call = pending_queue[idx].clone();
                idx += 1;

                self.emit(
                    &events_tx,
                    &ctx.session_id,
                    Some(&ctx.task_id),
                    StreamPayload::ToolCallEvent {
                        call_id: call.call_id.clone(),
                        tool_name: call.tool_name.clone(),
                        phase: ToolPhase::Started,
                        error: None,
                    },
                )
                .await;

                let outcome = self.execute_tool_checked(&ctx, &call).await;
                match outcome {
                    Ok(out) => {
                        tool_calls_completed = tool_calls_completed.saturating_add(1);
                        if !out.is_empty() {
                            outputs.push(out.clone());
                            self.emit(
                                &events_tx,
                                &ctx.session_id,
                                Some(&ctx.task_id),
                                StreamPayload::TaskChunk {
                                    chunk: out,
                                    is_stderr: false,
                                },
                            )
                            .await;
                        }
                        self.emit(
                            &events_tx,
                            &ctx.session_id,
                            Some(&ctx.task_id),
                            StreamPayload::ToolCallEvent {
                                call_id: call.call_id,
                                tool_name: call.tool_name,
                                phase: ToolPhase::Completed,
                                error: None,
                            },
                        )
                        .await;
                    }
                    Err(err) => {
                        tool_calls_failed = tool_calls_failed.saturating_add(1);
                        self.emit(
                            &events_tx,
                            &ctx.session_id,
                            Some(&ctx.task_id),
                            StreamPayload::ToolCallEvent {
                                call_id: call.call_id,
                                tool_name: call.tool_name,
                                phase: ToolPhase::Failed,
                                error: Some(err.to_string()),
                            },
                        )
                        .await;
                    }
                }
            }

            if turn_aborted || *cancel_rx.borrow() {
                final_stop = StopReason::Aborted;
                self.emit_turn_lifecycle(
                    &events_tx,
                    &ctx,
                    turns_executed,
                    TurnDecision::End,
                    StopReason::Aborted,
                )
                .await;
                break;
            }

            // Outer-loop check: dequeue any follow-up prompt before deciding to settle.
            if let Ok(follow_up) = follow_up_rx.try_recv() {
                follow_ups_applied = follow_ups_applied.saturating_add(1);
                pending_queue = self.planner.on_follow_up(&follow_up);
                self.emit_turn_lifecycle(
                    &events_tx,
                    &ctx,
                    turns_executed,
                    TurnDecision::Continue,
                    StopReason::ToolUse,
                )
                .await;
                continue;
            }

            final_stop = StopReason::EndTurn;
            self.emit_turn_lifecycle(
                &events_tx,
                &ctx,
                turns_executed,
                TurnDecision::End,
                StopReason::EndTurn,
            )
            .await;
            break;
        }

        Ok(LoopRunReport {
            turns_executed,
            tool_calls_completed,
            tool_calls_failed,
            steering_applied,
            follow_ups_applied,
            stop_reason: final_stop,
            final_output: outputs.join("\n"),
        })
    }

    async fn execute_tool_checked(
        &self,
        ctx: &LoopRunContext,
        call: &ToolInvocation,
    ) -> Result<String, BoxerError> {
        let Some(tool) = self.registry.get(&call.tool_name) else {
            return Err(BoxerError::ExecutionFailed(format!(
                "tool `{}` is not registered",
                call.tool_name
            )));
        };

        if !ctx.permissions.allowed_tools.is_empty()
            && !ctx
                .permissions
                .allowed_tools
                .iter()
                .any(|allowed| allowed == &call.tool_name)
        {
            return Err(BoxerError::IsolationViolation(format!(
                "tool `{}` is outside swarm PermissionMask allowlist",
                call.tool_name
            )));
        }

        if !ctx.permissions.can_exec_shell && !ctx.permissions.can_read_fs {
            return Err(BoxerError::IsolationViolation(
                "tool execution denied by swarm PermissionMask".into(),
            ));
        }

        if ctx.mode == InteractionMode::Plan && tool.is_mutating() {
            return Err(BoxerError::IsolationViolation(format!(
                "mutating tool `{}` is forbidden in Plan mode",
                call.tool_name
            )));
        }

        if tool.is_mutating() && !ctx.permissions.can_write_fs {
            return Err(BoxerError::IsolationViolation(
                "mutating tool denied: can_write_fs=false in PermissionMask".into(),
            ));
        }

        if tool.requires_network() && !ctx.permissions.can_network {
            return Err(BoxerError::IsolationViolation(
                "network tool denied: can_network=false in PermissionMask".into(),
            ));
        }

        tool.execute(call.input.clone()).await
    }

    async fn emit_turn_lifecycle(
        &self,
        tx: &mpsc::Sender<EventEnvelope>,
        ctx: &LoopRunContext,
        turn: u32,
        decision: TurnDecision,
        stop_reason: StopReason,
    ) {
        self.emit(
            tx,
            &ctx.session_id,
            Some(&ctx.task_id),
            StreamPayload::AgentTurnLifecycle {
                turn_id: format!("{}-t{turn}", ctx.task_id),
                decision,
                stop_reason,
            },
        )
        .await;
    }

    async fn emit(
        &self,
        tx: &mpsc::Sender<EventEnvelope>,
        session_id: &str,
        task_id: Option<&str>,
        payload: StreamPayload,
    ) {
        let env = EventEnvelope {
            seq: self.seq.fetch_add(1, Ordering::SeqCst),
            emitted_at_ms: now_ms(),
            codec: CodecId::EventsV1,
            session_id: session_id.to_owned(),
            task_id: task_id.map(ToOwned::to_owned),
            payload,
        };
        let _ = tx.send(env).await;
    }
}
