//! Inner parallel-safe tool batch execution, live steering interception,
//! Plan/Act enforcement, and structured tool error emission.

use std::sync::atomic::Ordering;

use agents_protocol::{
    CodecId, EventEnvelope, InteractionMode, PermissionMask, SteeringMessage, StopReason,
    StreamPayload, ToolPhase, TurnDecision,
};
use futures_util::future::join_all;
use tokio::sync::{mpsc, watch};

use crate::{
    config::now_ms,
    runtime::{
        engine::{NativeAgentRuntime, NativeRuntimeConfig, ToolCallRequest, ToolExecutionResult},
        session_tree::{SessionEntry, SessionTree},
    },
};

impl NativeAgentRuntime {
    #[allow(clippy::too_many_arguments)]
    pub(super) async fn execute_tool_batch(
        &self,
        cfg: &NativeRuntimeConfig,
        current_mode: &mut InteractionMode,
        tree: &mut SessionTree,
        calls: &[ToolCallRequest],
        events_tx: &mpsc::Sender<EventEnvelope>,
        steering_rx: &mut mpsc::Receiver<SteeringMessage>,
        cancel_rx: &watch::Receiver<bool>,
    ) -> (u32, bool) {
        let mut steering_count: u32 = 0;
        let mut idx = 0usize;

        while idx < calls.len() {
            if *cancel_rx.borrow() {
                return (steering_count, true);
            }

            if let Ok(steer) = steering_rx.try_recv() {
                steering_count = steering_count.saturating_add(1);
                if steer.content.eq_ignore_ascii_case("/plan") {
                    *current_mode = InteractionMode::Plan;
                } else if steer.content.eq_ignore_ascii_case("/act") {
                    *current_mode = InteractionMode::Act;
                }
                tree.append(SessionEntry::UserMessage {
                    text: format!("[steering] {}", steer.content),
                });
                self.emit_payload(
                    events_tx,
                    &tree.session_id,
                    Some(&cfg.task_id),
                    StreamPayload::SteeringAck {
                        message_id: steer.message_id,
                        accepted: true,
                    },
                )
                .await;
                break;
            }

            // Group contiguous parallel-safe read-only calls into one concurrent wave
            // while preserving deterministic source order in SessionTree!
            let can_parallelize = self.is_parallel_safe_call(&calls[idx], *current_mode, cfg);
            let wave_end = if can_parallelize {
                let mut end = idx + 1;
                while end < calls.len()
                    && self.is_parallel_safe_call(&calls[end], *current_mode, cfg)
                {
                    end += 1;
                }
                end
            } else {
                idx + 1
            };

            let wave = &calls[idx..wave_end];
            for call in wave {
                tree.append(SessionEntry::ToolCall {
                    call_id: call.call_id.clone(),
                    tool_name: call.tool_name.clone(),
                    input: call.input.clone(),
                });
                self.emit_payload(
                    events_tx,
                    &tree.session_id,
                    Some(&cfg.task_id),
                    StreamPayload::ToolCallEvent {
                        call_id: call.call_id.clone(),
                        tool_name: call.tool_name.clone(),
                        phase: ToolPhase::Started,
                        error: None,
                    },
                )
                .await;
            }

            let futures = wave
                .iter()
                .map(|call| self.run_single_tool(call, *current_mode, &cfg.permissions));
            let results = join_all(futures).await;

            for res in results {
                let phase = if res.is_error {
                    ToolPhase::Failed
                } else {
                    ToolPhase::Completed
                };
                let err_msg = if res.is_error {
                    Some(res.output.clone())
                } else {
                    None
                };
                tree.append(SessionEntry::ToolResult {
                    call_id: res.call_id.clone(),
                    tool_name: res.tool_name.clone(),
                    output: res.output,
                    is_error: res.is_error,
                });
                self.emit_payload(
                    events_tx,
                    &tree.session_id,
                    Some(&cfg.task_id),
                    StreamPayload::ToolCallEvent {
                        call_id: res.call_id,
                        tool_name: res.tool_name,
                        phase,
                        error: err_msg,
                    },
                )
                .await;
            }

            idx = wave_end;
        }

        (steering_count, false)
    }

    fn is_parallel_safe_call(
        &self,
        call: &ToolCallRequest,
        mode: InteractionMode,
        cfg: &NativeRuntimeConfig,
    ) -> bool {
        let Some(tool) = self.tools.get(&call.tool_name) else {
            return false;
        };
        let _ = (mode, cfg);
        tool.parallel_safe()
    }

    async fn run_single_tool(
        &self,
        call: &ToolCallRequest,
        mode: InteractionMode,
        permissions: &PermissionMask,
    ) -> ToolExecutionResult {
        let Some(tool) = self.tools.get(&call.tool_name) else {
            return ToolExecutionResult {
                call_id: call.call_id.clone(),
                tool_name: call.tool_name.clone(),
                output: format!("error: unknown tool `{}`", call.tool_name),
                is_error: true,
            };
        };

        if !permissions.allowed_tools.is_empty()
            && !permissions
                .allowed_tools
                .iter()
                .any(|t| t == &call.tool_name)
        {
            return ToolExecutionResult {
                call_id: call.call_id.clone(),
                tool_name: call.tool_name.clone(),
                output: format!("permission denied: `{}` outside allowlist", call.tool_name),
                is_error: true,
            };
        }

        if mode == InteractionMode::Plan && tool.is_mutating() {
            return ToolExecutionResult {
                call_id: call.call_id.clone(),
                tool_name: call.tool_name.clone(),
                output: format!(
                    "blocked: mutating tool `{}` disabled in Plan mode",
                    call.tool_name
                ),
                is_error: true,
            };
        }

        if tool.is_mutating() && !permissions.can_write_fs {
            return ToolExecutionResult {
                call_id: call.call_id.clone(),
                tool_name: call.tool_name.clone(),
                output: "permission denied: can_write_fs=false".to_owned(),
                is_error: true,
            };
        }

        match tool.execute(call.input.clone()).await {
            Ok(out) => ToolExecutionResult {
                call_id: call.call_id.clone(),
                tool_name: call.tool_name.clone(),
                output: out,
                is_error: false,
            },
            Err(err) => ToolExecutionResult {
                call_id: call.call_id.clone(),
                tool_name: call.tool_name.clone(),
                output: format!("tool error: {err}"),
                is_error: true,
            },
        }
    }

    pub(super) async fn emit_turn(
        &self,
        tx: &mpsc::Sender<EventEnvelope>,
        session_id: &str,
        task_id: &str,
        turn_index: u32,
        decision: TurnDecision,
        stop_reason: StopReason,
    ) {
        self.emit_payload(
            tx,
            session_id,
            Some(task_id),
            StreamPayload::AgentTurnLifecycle {
                turn_id: format!("{task_id}-t{turn_index}"),
                decision,
                stop_reason,
            },
        )
        .await;
    }

    pub(super) async fn emit_payload(
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
