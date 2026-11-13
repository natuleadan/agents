use std::{
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};

use agents_boxer::{
    artifacts::{ArtifactPackager, ArtifactQuota, sha256_hex},
    config::{BoxerCli, BoxerConfig, now_ms},
    daemon::{BoxerDaemon, InMemoryBoxerBus},
    error::BoxerError,
    leases::{BoxFuture, CleanupDropGuard, InMemoryLeaseClient, LeaseSupervisor},
    loop_engine::{
        BoxerTool, DualLoopEngine, LoopRunContext, ToolInvocation, ToolRegistry, TurnPlanner,
    },
    preview::{PreviewPublishRequest, TraefikFileProvider},
};
use agents_protocol::{
    ExecOptions, FollowUpMessage, InteractionMode, IsolationKind, PermissionMask, RuntimeKind,
    SandboxProfile, SetupMode, SteeringMessage, StopReason, StreamPayload, TaskRequest, TaskState,
    ToolPhase, TurnDecision, WorkerStatus,
};
use clap::Parser;
use serde_json::Value;
use tokio::sync::{mpsc, watch};

type TestResult = Result<(), Box<dyn std::error::Error>>;

fn unique_temp_dir(prefix: &str) -> Result<PathBuf, Box<dyn std::error::Error>> {
    let dir = std::env::temp_dir().join(format!("{prefix}-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir)?;
    Ok(dir)
}

#[test]
fn cleanup_drop_guard_runs_on_drop_and_panic_unwind() {
    let counter = Arc::new(AtomicUsize::new(0));
    {
        let c = Arc::clone(&counter);
        let _guard = CleanupDropGuard::new(c, |cnt| {
            cnt.fetch_add(1, Ordering::SeqCst);
        });
    }
    assert_eq!(counter.load(Ordering::SeqCst), 1);

    let c2 = Arc::clone(&counter);
    let panicked = std::panic::catch_unwind(move || {
        let _guard = CleanupDropGuard::new(c2, |cnt| {
            cnt.fetch_add(10, Ordering::SeqCst);
        });
        panic!("simulated task panic");
    });
    assert!(panicked.is_err());
    assert_eq!(counter.load(Ordering::SeqCst), 11);
}

#[tokio::test]
async fn lease_supervisor_rejects_concurrent_claims_and_trips_cancel_on_fencing_loss() -> TestResult
{
    let client = InMemoryLeaseClient::new();
    client.seed_task("task-fence-1", "sess-1").await;

    let (sup, mut cancel_rx) = LeaseSupervisor::acquire_and_start(
        client.clone(),
        "task-fence-1",
        "boxer-node-a",
        60,
        Duration::from_millis(30),
    )
    .await?;

    let conflict = LeaseSupervisor::acquire_and_start(
        client.clone(),
        "task-fence-1",
        "boxer-node-b",
        60,
        Duration::from_secs(1),
    )
    .await;
    assert!(matches!(conflict, Err(BoxerError::FencingConflict(_))));

    let _new_tok = client
        .force_bump_token("task-fence-1", "boxer-node-b")
        .await?;
    tokio::time::timeout(Duration::from_secs(2), cancel_rx.changed()).await??;
    assert!(*cancel_rx.borrow());
    assert!(sup.is_fencing_lost());
    sup.stop();
    Ok(())
}

#[test]
fn artifact_packager_computes_sha256_validates_and_enforces_quotas() -> TestResult {
    let root = unique_temp_dir("boxer-artifacts")?;
    std::fs::create_dir_all(root.join("assets"))?;
    let html_bytes = b"<html><body>Hello Boxer</body></html>";
    let js_bytes = b"console.log('ready');";
    std::fs::write(root.join("index.html"), html_bytes)?;
    std::fs::write(root.join("assets/app.js"), js_bytes)?;

    let packager = ArtifactPackager::default();
    let manifest = packager.package_directory("task-art-1", &root)?;
    manifest.validate()?;
    assert_eq!(manifest.files.len(), 2);
    assert_eq!(
        manifest.total_bytes,
        u64::try_from(html_bytes.len() + js_bytes.len())?
    );
    assert_eq!(manifest.files[0].path, "assets/app.js");
    assert_eq!(manifest.files[0].sha256, sha256_hex(js_bytes));
    assert_eq!(manifest.files[1].path, "index.html");
    assert_eq!(manifest.files[1].sha256, sha256_hex(html_bytes));
    assert_eq!(manifest.files[1].content_type, "text/html");

    let tight = ArtifactPackager::new(ArtifactQuota {
        max_files: 10,
        max_file_bytes: 16,
        max_total_bytes: 1024,
    });
    assert!(matches!(
        tight.package_directory("task-art-2", &root),
        Err(BoxerError::Artifact(_))
    ));

    let _ = std::fs::remove_dir_all(root);
    Ok(())
}

#[tokio::test]
async fn traefik_preview_enforces_vlan_writes_route_and_cleans_on_drop_and_ttl() -> TestResult {
    let dyn_dir = unique_temp_dir("boxer-traefik")?;
    let provider = TraefikFileProvider::new("preview.example.com", dyn_dir.clone())?;

    // Reject public upstream IP.
    let bad = provider
        .publish(PreviewPublishRequest {
            task_id: "task-p0".to_owned(),
            slug: "bad-app".to_owned(),
            internal_host: "8.8.8.8".to_owned(),
            internal_port: 8080,
            ttl_seconds: 60,
            preview_token: None,
        })
        .await;
    assert!(matches!(bad, Err(BoxerError::Preview(_))));

    // Valid private VLAN 10.x upstream writes YAML and deletes it automatically on guard drop.
    let route_file_path: PathBuf;
    {
        let (route, _drop_guard) = provider
            .publish(PreviewPublishRequest {
                task_id: "task-p1".to_owned(),
                slug: "My Interactive App!".to_owned(),
                internal_host: "10.12.0.4".to_owned(),
                internal_port: 3000,
                ttl_seconds: 60,
                preview_token: Some("preview-secret-123".to_owned()),
            })
            .await?;

        assert!(route.requires_token);
        assert!(route.slug.starts_with("my-interactive-app-"));
        assert!(route.public_url.ends_with(".preview.example.com"));
        route_file_path = provider.route_file_path(&route.route_id);
        assert!(route_file_path.exists());
        let yaml = std::fs::read_to_string(&route_file_path)?;
        assert!(yaml.contains("http://10.12.0.4:3000"));
        assert!(yaml.contains("X-Preview-Token-Sha256"));
    }
    assert!(
        !route_file_path.exists(),
        "DropGuard must delete preview route file on drop"
    );

    // TTL expiration sweep removes expired routes cleanly.
    let (ttl_route, ttl_guard) = provider
        .publish(PreviewPublishRequest {
            task_id: "task-p2".to_owned(),
            slug: "ttl-app".to_owned(),
            internal_host: "127.0.0.1".to_owned(),
            internal_port: 4000,
            ttl_seconds: 5,
            preview_token: None,
        })
        .await?;
    let ttl_path = provider.route_file_path(&ttl_route.route_id);
    assert!(ttl_path.exists());
    let swept = provider
        .sweep_expired(ttl_route.created_at_ms.saturating_add(10_000))
        .await?;
    assert_eq!(swept, 1);
    assert!(!ttl_path.exists());
    let _ = ttl_guard.dismiss();

    let _ = std::fs::remove_dir_all(dyn_dir);
    Ok(())
}

struct MockReadTool;
impl BoxerTool for MockReadTool {
    fn name(&self) -> &str {
        "read_note"
    }
    fn execute<'a>(&'a self, input: Value) -> BoxFuture<'a, Result<String, BoxerError>> {
        Box::pin(async move {
            Ok(format!(
                "read:{}",
                input
                    .get("target")
                    .and_then(Value::as_str)
                    .unwrap_or("default")
            ))
        })
    }
}

struct MockWriteTool;
impl BoxerTool for MockWriteTool {
    fn name(&self) -> &str {
        "write_note"
    }
    fn is_mutating(&self) -> bool {
        true
    }
    fn execute<'a>(&'a self, input: Value) -> BoxFuture<'a, Result<String, BoxerError>> {
        Box::pin(async move {
            Ok(format!(
                "wrote:{}",
                input
                    .get("target")
                    .and_then(Value::as_str)
                    .unwrap_or("file")
            ))
        })
    }
}

struct MockPlanner;
impl TurnPlanner for MockPlanner {
    fn plan_initial(&self, _prompt: &str) -> Vec<ToolInvocation> {
        vec![
            ToolInvocation {
                call_id: "c1".to_owned(),
                tool_name: "read_note".to_owned(),
                input: serde_json::json!({ "target": "spec" }),
            },
            ToolInvocation {
                call_id: "c2".to_owned(),
                tool_name: "write_note".to_owned(),
                input: serde_json::json!({ "target": "app.html" }),
            },
        ]
    }
    fn on_steering(&self, steering: &SteeringMessage) -> Vec<ToolInvocation> {
        vec![ToolInvocation {
            call_id: "c-steer".to_owned(),
            tool_name: "read_note".to_owned(),
            input: serde_json::json!({ "target": steering.content }),
        }]
    }
    fn on_follow_up(&self, follow_up: &FollowUpMessage) -> Vec<ToolInvocation> {
        vec![ToolInvocation {
            call_id: "c-follow".to_owned(),
            tool_name: "write_note".to_owned(),
            input: serde_json::json!({ "target": follow_up.content }),
        }]
    }
}

#[tokio::test]
async fn dual_loop_enforces_plan_mode_and_processes_steering_and_followups() -> TestResult {
    let mut reg = ToolRegistry::new();
    reg.register(MockReadTool);
    reg.register(MockWriteTool);
    let engine = DualLoopEngine::new(Arc::new(reg), Arc::new(MockPlanner));

    // 1) Plan mode blocks mutating tools fail-closed.
    let (ev_tx, mut ev_rx) = mpsc::channel(64);
    let (_st_tx, st_rx) = mpsc::channel(8);
    let (_fu_tx, fu_rx) = mpsc::channel(8);
    let (_cx_tx, cx_rx) = watch::channel(false);
    let plan_report = engine
        .run(
            LoopRunContext {
                session_id: "s-plan".to_owned(),
                task_id: "t-plan".to_owned(),
                initial_prompt: "inspect".to_owned(),
                mode: InteractionMode::Plan,
                permissions: PermissionMask {
                    can_read_fs: true,
                    can_write_fs: true,
                    can_exec_shell: true,
                    can_network: false,
                    allowed_tools: Vec::new(),
                },
                max_turns: 4,
            },
            ev_tx,
            st_rx,
            fu_rx,
            cx_rx,
        )
        .await?;
    assert_eq!(plan_report.tool_calls_completed, 1);
    assert_eq!(plan_report.tool_calls_failed, 1);
    let mut saw_plan_failure = false;
    while let Ok(env) = ev_rx.try_recv() {
        if let StreamPayload::ToolCallEvent {
            phase: ToolPhase::Failed,
            ..
        } = env.payload
        {
            saw_plan_failure = true;
        }
    }
    assert!(saw_plan_failure);

    // 2) Act mode drains live steering + outer follow-up with TurnDecision::Continue -> End.
    let (ev_tx2, mut ev_rx2) = mpsc::channel(64);
    let (st_tx2, st_rx2) = mpsc::channel(8);
    let (fu_tx2, fu_rx2) = mpsc::channel(8);
    let (_cx_tx2, cx_rx2) = watch::channel(false);

    st_tx2
        .send(SteeringMessage {
            message_id: "m-steer-1".to_owned(),
            session_id: "s-act".to_owned(),
            task_id: "t-act".to_owned(),
            content: "steered-target".to_owned(),
            emitted_at_ms: now_ms(),
        })
        .await?;
    fu_tx2
        .send(FollowUpMessage {
            message_id: "m-follow-1".to_owned(),
            session_id: "s-act".to_owned(),
            content: "followup.html".to_owned(),
            queued_at_ms: now_ms(),
        })
        .await?;

    let act_report = engine
        .run(
            LoopRunContext {
                session_id: "s-act".to_owned(),
                task_id: "t-act".to_owned(),
                initial_prompt: "build".to_owned(),
                mode: InteractionMode::Act,
                permissions: PermissionMask {
                    can_read_fs: true,
                    can_write_fs: true,
                    can_exec_shell: true,
                    can_network: false,
                    allowed_tools: Vec::new(),
                },
                max_turns: 4,
            },
            ev_tx2,
            st_rx2,
            fu_rx2,
            cx_rx2,
        )
        .await?;

    assert_eq!(act_report.steering_applied, 1);
    assert_eq!(act_report.follow_ups_applied, 1);
    assert_eq!(act_report.turns_executed, 2);
    assert_eq!(act_report.stop_reason, StopReason::EndTurn);

    let mut saw_steer_ack = false;
    let mut saw_continue = false;
    while let Ok(env) = ev_rx2.try_recv() {
        match env.payload {
            StreamPayload::SteeringAck { accepted: true, .. } => saw_steer_ack = true,
            StreamPayload::AgentTurnLifecycle {
                decision: TurnDecision::Continue,
                ..
            } => saw_continue = true,
            _ => {}
        }
    }
    assert!(saw_steer_ack);
    assert!(saw_continue);
    Ok(())
}

#[tokio::test]
async fn boxer_daemon_executes_hermetic_task_emits_telemetry_and_completes_fenced() -> TestResult {
    let ws_root = unique_temp_dir("boxer-daemon-ws")?;
    let dyn_dir = unique_temp_dir("boxer-daemon-traefik")?;

    let mut cli = BoxerCli::parse_from([
        "agents-boxer",
        "--node-id",
        "boxer-hermetic-01",
        "--runtime",
        "host",
        "--isolation",
        "none",
    ]);
    cli.workspace_root = ws_root.clone();
    cli.traefik_dynamic_dir = dyn_dir.clone();

    let cfg = BoxerConfig::from_cli(cli)?;
    let lease_client = InMemoryLeaseClient::new();
    lease_client.seed_task("task-e2e-1", "sess-e2e").await;
    let bus = InMemoryBoxerBus::new();

    let daemon = BoxerDaemon::new(cfg, lease_client.clone(), bus.clone())?;
    let res = daemon
        .execute_task(
            TaskRequest {
                task_id: "task-e2e-1".to_owned(),
                session_id: "sess-e2e".to_owned(),
                prompt: "printf '<h1>App</h1>' > index.html && echo built".to_owned(),
                timeout_seconds: Some(15),
                mode: InteractionMode::Act,
                fencing_token: None,
                idempotency_key: None,
                sandbox: Some(SandboxProfile {
                    mode: SetupMode::Local,
                    runtime: RuntimeKind::Host,
                    isolation: IsolationKind::None,
                    allow_host_native: false,
                    gateway_url: None,
                }),
                exec: ExecOptions::default(),
            },
            None,
        )
        .await?;

    assert_eq!(res.exit_code, 0);
    assert_eq!(res.stop_reason, StopReason::EndTurn);
    assert!(
        res.output_summary
            .as_deref()
            .is_some_and(|s| s.contains("built"))
    );

    let completion = lease_client.recorded_completion("task-e2e-1").await;
    assert!(matches!(
        completion,
        Some((TaskState::Done, Some(0), Some(StopReason::EndTurn), _))
    ));

    let events = bus.recorded_events().await;
    assert!(events.iter().any(|e| matches!(
        e.payload,
        StreamPayload::WorkerStateChanged {
            status: WorkerStatus::Working,
            ..
        }
    )));
    assert!(events.iter().any(|e| matches!(
        e.payload,
        StreamPayload::TaskChunk { ref chunk, .. } if chunk == "built"
    )));
    assert!(events.iter().any(|e| matches!(
        e.payload,
        StreamPayload::WorkerStateChanged {
            status: WorkerStatus::Done,
            ..
        }
    )));
    // Sequence numbers strictly monotonic.
    for pair in events.windows(2) {
        assert!(pair[0].seq < pair[1].seq);
    }

    let _ = std::fs::remove_dir_all(ws_root);
    let _ = std::fs::remove_dir_all(dyn_dir);
    Ok(())
}
