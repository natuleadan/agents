use std::{
    collections::BTreeMap,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};

use agents_boxer::{
    config::now_ms,
    error::BoxerError,
    leases::BoxFuture,
    runtime::{
        DefaultLoopHooks, LoopHooks, ModelTurnResponse, NativeAgentRuntime, NativeRuntimeConfig,
        OutputSchemaContract, RetryPolicy, RuntimeTool, ScriptedModelClient, SessionEntry,
        SessionTree, ToolCallRequest,
    },
};
use agents_protocol::{
    FollowUpMessage, InteractionMode, PermissionMask, SteeringMessage, StopReason, StreamPayload,
    TurnDecision,
};
use serde_json::{Value, json};
use tokio::sync::{mpsc, watch};

type TestResult = Result<(), Box<dyn std::error::Error>>;

fn full_mask() -> PermissionMask {
    PermissionMask {
        can_read_fs: true,
        can_write_fs: true,
        can_exec_shell: true,
        can_network: true,
        allowed_tools: Vec::new(),
    }
}

struct TimedEchoTool {
    tool_name: &'static str,
    delay: Duration,
    mutating: bool,
    calls: Arc<AtomicUsize>,
}

impl RuntimeTool for TimedEchoTool {
    fn name(&self) -> &str {
        self.tool_name
    }

    fn is_mutating(&self) -> bool {
        self.mutating
    }

    fn execute<'a>(&'a self, input: Value) -> BoxFuture<'a, Result<String, BoxerError>> {
        Box::pin(async move {
            self.calls.fetch_add(1, Ordering::SeqCst);
            if !self.delay.is_zero() {
                tokio::time::sleep(self.delay).await;
            }
            if input.get("fail").and_then(Value::as_bool) == Some(true) {
                return Err(BoxerError::ExecutionFailed("simulated tool failure".into()));
            }
            Ok(format!(
                "{}:{}",
                self.tool_name,
                input.get("tag").and_then(Value::as_str).unwrap_or("ok")
            ))
        })
    }
}

struct ContinueOnceHook {
    remaining_continues: AtomicUsize,
}

impl LoopHooks for ContinueOnceHook {
    fn prepare_next_turn<'a>(
        &'a self,
        _turn_index: u32,
        _tree: &'a SessionTree,
        pending_follow_up: Option<&'a FollowUpMessage>,
    ) -> BoxFuture<'a, TurnDecision> {
        Box::pin(async move {
            if pending_follow_up.is_some()
                || self
                    .remaining_continues
                    .try_update(Ordering::SeqCst, Ordering::SeqCst, |v| v.checked_sub(1))
                    .is_ok()
            {
                TurnDecision::Continue
            } else {
                TurnDecision::End
            }
        })
    }
}

#[tokio::test]
async fn live_steering_interrupts_inner_batch_while_hooks_and_followup_drive_outer_turn()
-> TestResult {
    let model = ScriptedModelClient::new(vec![
        Ok(ModelTurnResponse {
            assistant_text: "running tools".to_owned(),
            tool_calls: vec![
                ToolCallRequest {
                    call_id: "c1".to_owned(),
                    tool_name: "read_a".to_owned(),
                    input: json!({ "tag": "1" }),
                },
                ToolCallRequest {
                    call_id: "c2".to_owned(),
                    tool_name: "write_b".to_owned(),
                    input: json!({ "tag": "2" }),
                },
            ],
            stop_reason: StopReason::ToolUse,
            usage_tokens: 10,
        }),
        Ok(ModelTurnResponse {
            assistant_text: "after steering continue".to_owned(),
            tool_calls: Vec::new(),
            stop_reason: StopReason::EndTurn,
            usage_tokens: 5,
        }),
        Ok(ModelTurnResponse {
            assistant_text: "after followup".to_owned(),
            tool_calls: Vec::new(),
            stop_reason: StopReason::EndTurn,
            usage_tokens: 5,
        }),
    ]);

    let read_calls = Arc::new(AtomicUsize::new(0));
    let write_calls = Arc::new(AtomicUsize::new(0));
    let mut rt = NativeAgentRuntime::new(
        model.clone(),
        Arc::new(ContinueOnceHook {
            remaining_continues: AtomicUsize::new(0),
        }),
    );
    rt.register_tool(TimedEchoTool {
        tool_name: "read_a",
        delay: Duration::ZERO,
        mutating: false,
        calls: Arc::clone(&read_calls),
    });
    rt.register_tool(TimedEchoTool {
        tool_name: "write_b",
        delay: Duration::ZERO,
        mutating: true,
        calls: Arc::clone(&write_calls),
    });

    let cfg = NativeRuntimeConfig {
        task_id: "task-steer".to_owned(),
        endpoint_id: "provider_endpoint:primary".to_owned(),
        mode: InteractionMode::Act,
        permissions: full_mask(),
        max_turns: 8,
        compact_keep_tail: 20,
        retry_policy: RetryPolicy::default(),
        output_contract: None,
    };

    let mut tree = SessionTree::new("sess-steer");
    tree.append(SessionEntry::UserMessage {
        text: "start task".to_owned(),
    });

    let (ev_tx, mut ev_rx) = mpsc::channel(64);
    let (steer_tx, steer_rx) = mpsc::channel(8);
    let (fu_tx, fu_rx) = mpsc::channel(8);
    let (_cancel_tx, cancel_rx) = watch::channel(false);

    // Pre-enqueue live steering (`/plan`) and a follow-up for the outer loop.
    steer_tx
        .send(SteeringMessage {
            message_id: "st-1".to_owned(),
            session_id: "sess-steer".to_owned(),
            task_id: "task-steer".to_owned(),
            content: "/plan".to_owned(),
            emitted_at_ms: now_ms(),
        })
        .await?;
    fu_tx
        .send(FollowUpMessage {
            message_id: "fu-1".to_owned(),
            session_id: "sess-steer".to_owned(),
            content: "now summarize".to_owned(),
            queued_at_ms: now_ms(),
        })
        .await?;

    let outcome = rt
        .run(&cfg, &mut tree, ev_tx, steer_rx, fu_rx, cancel_rx)
        .await?;

    // Steering popped at batch start aborted both tool executions cleanly.
    assert_eq!(read_calls.load(Ordering::SeqCst), 0);
    assert_eq!(write_calls.load(Ordering::SeqCst), 0);
    assert_eq!(outcome.steering_acks, 1);
    assert_eq!(outcome.turns_executed, 3);
    assert_eq!(outcome.final_text, "after followup");

    let mut events = Vec::new();
    while let Ok(env) = ev_rx.try_recv() {
        events.push(env.payload);
    }
    assert!(events.iter().any(|p| matches!(
        p,
        StreamPayload::SteeringAck {
            message_id,
            accepted: true
        } if message_id == "st-1"
    )));
    Ok(())
}

#[tokio::test]
async fn parallel_safe_tools_preserve_source_order_and_length_aborts_entire_batch() -> TestResult {
    let model = ScriptedModelClient::new(vec![
        Ok(ModelTurnResponse {
            assistant_text: "parallel read".to_owned(),
            tool_calls: vec![
                ToolCallRequest {
                    call_id: "slow".to_owned(),
                    tool_name: "slow_read".to_owned(),
                    input: json!({ "tag": "first" }),
                },
                ToolCallRequest {
                    call_id: "fast".to_owned(),
                    tool_name: "fast_read".to_owned(),
                    input: json!({ "tag": "second" }),
                },
            ],
            stop_reason: StopReason::ToolUse,
            usage_tokens: 10,
        }),
        Ok(ModelTurnResponse {
            assistant_text: "truncated output".to_owned(),
            tool_calls: vec![ToolCallRequest {
                call_id: "never".to_owned(),
                tool_name: "fast_read".to_owned(),
                input: json!({ "tag": "blocked" }),
            }],
            stop_reason: StopReason::Length,
            usage_tokens: 10,
        }),
    ]);

    let slow_calls = Arc::new(AtomicUsize::new(0));
    let fast_calls = Arc::new(AtomicUsize::new(0));
    let mut rt = NativeAgentRuntime::new(model, Arc::new(DefaultLoopHooks));
    rt.register_tool(TimedEchoTool {
        tool_name: "slow_read",
        delay: Duration::from_millis(35),
        mutating: false,
        calls: Arc::clone(&slow_calls),
    });
    rt.register_tool(TimedEchoTool {
        tool_name: "fast_read",
        delay: Duration::from_millis(5),
        mutating: false,
        calls: Arc::clone(&fast_calls),
    });

    let cfg = NativeRuntimeConfig {
        task_id: "task-par".to_owned(),
        endpoint_id: "provider_endpoint:primary".to_owned(),
        mode: InteractionMode::Act,
        permissions: full_mask(),
        max_turns: 4,
        compact_keep_tail: 20,
        retry_policy: RetryPolicy::default(),
        output_contract: None,
    };

    let mut tree = SessionTree::new("sess-par");
    let (ev_tx, _ev_rx) = mpsc::channel(64);
    let (_st_tx, st_rx) = mpsc::channel(4);
    let (_fu_tx, fu_rx) = mpsc::channel(4);
    let (_c_tx, c_rx) = watch::channel(false);

    let outcome = rt.run(&cfg, &mut tree, ev_tx, st_rx, fu_rx, c_rx).await?;
    assert_eq!(outcome.stop_reason, StopReason::Length);
    assert_eq!(outcome.aborted_length_batches, 1);
    assert_eq!(slow_calls.load(Ordering::SeqCst), 1);
    assert_eq!(fast_calls.load(Ordering::SeqCst), 1);

    let result_ids: Vec<String> = tree
        .active_entries()
        .into_iter()
        .filter_map(|e| match e {
            SessionEntry::ToolResult { call_id, .. } => Some(call_id),
            _ => None,
        })
        .collect();
    assert_eq!(result_ids, vec!["slow".to_owned(), "fast".to_owned()]);
    Ok(())
}

#[tokio::test]
async fn tool_errors_and_retries_omit_failed_attempts_from_model_context() -> TestResult {
    let model = ScriptedModelClient::new(vec![
        Err(BoxerError::ExecutionFailed("transient upstream 503".into())),
        Ok(ModelTurnResponse {
            assistant_text: "calling bad tools".to_owned(),
            tool_calls: vec![
                ToolCallRequest {
                    call_id: "u1".to_owned(),
                    tool_name: "does_not_exist".to_owned(),
                    input: json!({}),
                },
                ToolCallRequest {
                    call_id: "f1".to_owned(),
                    tool_name: "read_tool".to_owned(),
                    input: json!({ "fail": true }),
                },
            ],
            stop_reason: StopReason::ToolUse,
            usage_tokens: 8,
        }),
        Ok(ModelTurnResponse {
            assistant_text: r#"{"status":"recovered","ok":true}"#.to_owned(),
            tool_calls: Vec::new(),
            stop_reason: StopReason::EndTurn,
            usage_tokens: 9,
        }),
    ]);

    let calls = Arc::new(AtomicUsize::new(0));
    let mut rt = NativeAgentRuntime::new(model.clone(), Arc::new(DefaultLoopHooks));
    rt.register_tool(TimedEchoTool {
        tool_name: "read_tool",
        delay: Duration::ZERO,
        mutating: false,
        calls,
    });

    let cfg = NativeRuntimeConfig {
        task_id: "task-retry".to_owned(),
        endpoint_id: "provider_endpoint:primary".to_owned(),
        mode: InteractionMode::Plan,
        permissions: full_mask(),
        max_turns: 4,
        compact_keep_tail: 20,
        retry_policy: RetryPolicy {
            max_retries: 2,
            base_delay_ms: 1,
            max_delay_ms: 5,
        },
        output_contract: Some(OutputSchemaContract {
            required_fields: vec!["status".to_owned(), "ok".to_owned()],
        }),
    };

    let mut tree = SessionTree::new("sess-retry");
    tree.append(SessionEntry::UserMessage {
        text: "run safely".to_owned(),
    });
    let (ev_tx, _ev_rx) = mpsc::channel(64);
    let (_st_tx, st_rx) = mpsc::channel(4);
    let (_fu_tx, fu_rx) = mpsc::channel(4);
    let (_c_tx, c_rx) = watch::channel(false);

    let outcome = rt.run(&cfg, &mut tree, ev_tx, st_rx, fu_rx, c_rx).await?;
    assert_eq!(outcome.stop_reason, StopReason::EndTurn);
    assert_eq!(outcome.retry_audit.len(), 1);
    assert!(outcome.retry_audit[0].omitted_from_context);
    assert_eq!(
        outcome.structured_json,
        Some(json!({ "status": "recovered", "ok": true }))
    );

    // Ensure seen requests to the model never contain retry error strings.
    let seen = model.seen_requests().await;
    for req in &seen {
        for entry in &req.entries {
            assert!(!entry.summary_text().contains("transient upstream 503"));
        }
    }

    // Both tool errors were fed cleanly into SessionTree as structured error ToolResult entries.
    let error_results: Vec<String> = tree
        .active_entries()
        .into_iter()
        .filter_map(|e| match e {
            SessionEntry::ToolResult {
                call_id,
                is_error: true,
                ..
            } => Some(call_id),
            _ => None,
        })
        .collect();
    assert_eq!(error_results, vec!["u1".to_owned(), "f1".to_owned()]);
    Ok(())
}

#[test]
fn compaction_never_splits_tool_call_and_result_pairs_across_branches() -> TestResult {
    let mut tree = SessionTree::new("sess-compact");
    tree.append(SessionEntry::UserMessage {
        text: "msg-1".to_owned(),
    });
    tree.append(SessionEntry::ToolCall {
        call_id: "c1".to_owned(),
        tool_name: "grep".to_owned(),
        input: json!({ "q": "foo" }),
    });
    let after_c1 = tree.append(SessionEntry::ToolResult {
        call_id: "c1".to_owned(),
        tool_name: "grep".to_owned(),
        output: "match".to_owned(),
        is_error: false,
    });
    tree.append(SessionEntry::AssistantText {
        text: "done".to_owned(),
    });
    tree.append(SessionEntry::ToolCall {
        call_id: "c2".to_owned(),
        tool_name: "read".to_owned(),
        input: json!({ "p": "lib.rs" }),
    });
    tree.append(SessionEntry::ToolResult {
        call_id: "c2".to_owned(),
        tool_name: "read".to_owned(),
        output: "fn main(){}".to_owned(),
        is_error: false,
    });
    tree.append(SessionEntry::UserMessage {
        text: "next".to_owned(),
    });

    for keep in 1..=7 {
        let view = tree.compact_for_request(keep);
        for (idx, entry) in view.iter().enumerate() {
            if let SessionEntry::ToolResult { call_id, .. } = entry {
                assert!(idx > 0, "ToolResult `{call_id}` orphaned at start");
                let prev = &view[idx - 1];
                assert!(
                    matches!(prev, SessionEntry::ToolCall { call_id: prev_id, .. } if prev_id == call_id),
                    "ToolResult `{call_id}` split from its ToolCall when keep={keep}"
                );
            }
        }
    }

    // Fork branch from `after_c1`, create checkpoint and provenance memory.
    tree.fork_branch("alt-branch", Some(&after_c1));
    let mut snaps = BTreeMap::new();
    snaps.insert("src/main.rs".to_owned(), Some(b"fn main() {}".to_vec()));
    let cp_id = tree.record_checkpoint("before-edit", snaps);
    assert!(tree.get_checkpoint(&cp_id).is_some());

    let mem = tree.add_memory(
        "Prefer cargo fmt before validate",
        "ci_failure_turn_3",
        Some("rust-hygiene".to_owned()),
        92,
    );
    assert_eq!(mem.provenance_source, "ci_failure_turn_3");
    assert_eq!(tree.memories().len(), 1);
    assert!(tree.checkout_branch("main"));
    Ok(())
}
