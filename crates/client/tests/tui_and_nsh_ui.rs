#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::sync::Arc;

use agents_client::core::adapters::NativeReqwestTransport;
use agents_client::core::{
    ActiveView, AppAction, AppState, AttachmentEntry, BackendTarget, ContextMeter, DiffHunk,
    DiffLine, DiffLineKind, InMemorySecretStore, ServerModeProvider, SubagentDrawerMessage,
    SwarmForkEntry, ThinkingEntry, UpdateInfo, WorkerEntry, dispatch,
};
use agents_client::styles::{SurfaceMetrics, TargetPlatform};
use agents_client::tui::{TuiKey, map_key, render_tui};
use agents_client::ui::{ClientController, render_app_shell};
use agents_protocol::{
    CodecId, EventEnvelope, InteractionMode, SetupMode, StreamPayload, ThirdPartyProvider,
    TraefikPreviewRoute, WorkerStatus,
};
use agents_server::{InfrastructureHandles, ServerConfig, build_application};
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use serde_json::json;
use tokio::net::TcpListener;

fn buffer_text(terminal: &Terminal<TestBackend>) -> String {
    let buf = terminal.backend().buffer();
    let mut out = String::new();
    for y in 0..buf.area.height {
        for x in 0..buf.area.width {
            out.push_str(buf[(x, y)].symbol());
        }
        out.push('\n');
    }
    out
}

#[test]
fn ratatui_renders_onboarding_workers_diffs_thinking_meter_and_palette() {
    let mut state = AppState::default();
    dispatch(
        &mut state,
        AppAction::SetWorker(WorkerEntry {
            id: "w-1".to_owned(),
            role: "coder".to_owned(),
            status: WorkerStatus::Working,
            current_task: "Writing UI".to_owned(),
        }),
    );
    dispatch(
        &mut state,
        AppAction::AddThinkingStep(ThinkingEntry {
            id: "th-1".to_owned(),
            title: "Plan tokens".to_owned(),
            detail: "Mapped light/dark contrast pairs".to_owned(),
            collapsed: false,
        }),
    );
    dispatch(
        &mut state,
        AppAction::AddDiff(DiffHunk {
            file_path: "src/lib.rs".to_owned(),
            header: "@@ -1,1 +1,2 @@".to_owned(),
            lines: vec![
                DiffLine {
                    kind: DiffLineKind::Removed,
                    content: "old_call()".to_owned(),
                },
                DiffLine {
                    kind: DiffLineKind::Added,
                    content: "new_call()".to_owned(),
                },
            ],
        }),
    );
    dispatch(
        &mut state,
        AppAction::UpdateContextMeter(ContextMeter {
            used_tokens: 45_000,
            max_tokens: 200_000,
            estimated_cost_usd: 0.1425,
        }),
    );

    let backend = TestBackend::new(110, 28);
    let mut terminal = Terminal::new(backend).unwrap();
    render_tui(&mut terminal, &state).unwrap();

    let frame_one = buffer_text(&terminal);
    assert!(frame_one.contains("local · docker + openshell"));
    assert!(frame_one.contains("[WORKING] coder: Writing UI"));
    assert!(frame_one.contains("Mapped light/dark contrast pairs"));
    assert!(frame_one.contains("+new_call()"));
    assert!(frame_one.contains("45000/200000 tok"));

    // Keyboard navigation: toggle thinking, switch setup mode, open Command Palette.
    dispatch(
        &mut state,
        AppAction::ToggleThinkingCollapse("th-1".to_owned()),
    );
    if let Some(a) = map_key(&state, TuiKey::Char('2')) {
        dispatch(&mut state, a);
    }
    assert_eq!(state.setup_mode, SetupMode::Remote);
    if let Some(a) = map_key(&state, TuiKey::CtrlK) {
        dispatch(&mut state, a);
    }
    assert_eq!(state.active_view, ActiveView::CommandPalette);

    render_tui(&mut terminal, &state).unwrap();
    let frame_two = buffer_text(&terminal);
    assert!(frame_two.contains("remote · server"));
    assert!(frame_two.contains("Command Palette"));
    assert!(!frame_two.contains("Mapped light/dark contrast pairs"));
}

#[test]
fn nsh_ui_adapts_across_desktop_mobile_and_web_with_badges_and_drawers() {
    let mut state = AppState::default();
    let daytona = BackendTarget::third_party(ThirdPartyProvider::Daytona, "https://daytona.io");
    dispatch(
        &mut state,
        AppAction::UpdateEffectiveProfile(daytona.effective_profile()),
    );
    dispatch(
        &mut state,
        AppAction::AddAttachment(AttachmentEntry {
            name: "screen.png".to_owned(),
            mime_type: "image/png".to_owned(),
            bytes: 2048,
        }),
    );
    dispatch(
        &mut state,
        AppAction::PushSubagentMessage(SubagentDrawerMessage {
            id: "s-1".to_owned(),
            from_role: "reviewer".to_owned(),
            summary: "Approved migration diff".to_owned(),
            requires_approval: false,
        }),
    );
    dispatch(
        &mut state,
        AppAction::UpsertSwarmFork(SwarmForkEntry {
            id: "f-1".to_owned(),
            branch: "feat/ui".to_owned(),
            parent_branch: "dev".to_owned(),
            status: WorkerStatus::Done,
            changed_files: 6,
        }),
    );
    dispatch(
        &mut state,
        AppAction::SetUpdateInfo(UpdateInfo {
            current_version: "0.1.0".to_owned(),
            available_version: Some("0.1.1".to_owned()),
            notes: "Performance polish".to_owned(),
        }),
    );
    dispatch(
        &mut state,
        AppAction::RingAttention("Approve tool execution".to_owned()),
    );

    let ios_tree = render_app_shell(&state, SurfaceMetrics::mobile(TargetPlatform::Ios));
    let ios_html = ios_tree.render_html_fragment();
    assert!(ios_html.contains("remote · daytona"));
    assert!(ios_html.contains("mobile-shell"));
    assert!(ios_html.contains("role=\"sheet\""));
    assert!(ios_html.contains("Approved migration diff"));
    assert!(ios_html.contains("0.1.0 -&gt; 0.1.1"));

    let mac_tree = render_app_shell(&state, SurfaceMetrics::desktop(TargetPlatform::MacOs));
    let mac_html = mac_tree.render_html_fragment();
    assert!(mac_html.contains("desktop-shell"));
    assert!(mac_html.contains("role=\"navigation\""));

    let web_tree = render_app_shell(&state, SurfaceMetrics::web(1440.0, 900.0));
    assert!(
        web_tree
            .flatten_text_lines()
            .iter()
            .any(|l| l.contains("target:web (Expanded)"))
    );
}

#[tokio::test]
async fn end_to_end_login_session_prompt_stream_and_live_preview_against_server() {
    let mut config = ServerConfig::for_tests();
    config.rate_limit_max_requests = 1000;
    let handles = InfrastructureHandles::from_config(config).await.unwrap();
    let app = build_application(handles).await.unwrap();
    let router = app.platform().router().clone();

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server_task = tokio::spawn(async move {
        let _ = axum::serve(listener, router).await;
    });

    let base_url = format!("http://{addr}");
    // Register a real user against the running agents-server HTTP listener.
    let raw_http = reqwest::Client::new();
    let reg = raw_http
        .post(format!("{base_url}/api/v1/auth/register"))
        .json(&json!({
            "email": "e2e-client@example.com",
            "password": "very-secure-password-123"
        }))
        .send()
        .await
        .unwrap();
    assert!(reg.status().is_success());

    let transport = Arc::new(NativeReqwestTransport::default());
    let provider = Arc::new(ServerModeProvider::new(&base_url, transport));
    let secrets = Arc::new(InMemorySecretStore::default());

    let mut controller =
        ClientController::new(provider, secrets, BackendTarget::local_server(&base_url));
    controller
        .login_and_persist("e2e-client@example.com", "very-secure-password-123")
        .await
        .unwrap();

    let session = controller
        .start_session("E2E Live App", InteractionMode::Act)
        .await
        .unwrap();
    let task = controller
        .submit_prompt("Build preview landing page")
        .await
        .unwrap();

    controller.ingest_stream_envelope(EventEnvelope {
        seq: 1,
        emitted_at_ms: 100,
        codec: CodecId::EventsV1,
        session_id: session.id.clone(),
        task_id: Some(task.task_id.clone()),
        payload: StreamPayload::TaskChunk {
            chunk: "Preview compiled cleanly.".to_owned(),
            is_stderr: false,
        },
    });
    controller.ingest_stream_envelope(EventEnvelope {
        seq: 2,
        emitted_at_ms: 101,
        codec: CodecId::EventsV1,
        session_id: session.id,
        task_id: Some(task.task_id.clone()),
        payload: StreamPayload::PreviewPublished(TraefikPreviewRoute {
            route_id: "rt-1".to_owned(),
            task_id: task.task_id,
            slug: "landing-preview".to_owned(),
            internal_host: "127.0.0.1".to_owned(),
            internal_port: 3000,
            public_url: "https://preview-1.localhost/".to_owned(),
            ttl_seconds: 900,
            requires_token: false,
            created_at_ms: 101,
        }),
    });

    let tree = controller.render(SurfaceMetrics::desktop(TargetPlatform::Linux));
    let lines = tree.flatten_text_lines();
    assert!(lines.iter().any(|l| l == "Preview compiled cleanly."));
    assert!(lines.iter().any(|l| l == "https://preview-1.localhost/"));
    assert_eq!(controller.state.notifications.completion_chimes, 1);

    server_task.abort();
}
