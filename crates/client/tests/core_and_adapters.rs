#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::sync::Arc;

use agents_client::core::{
    BackendTarget, ChunkCoalescer, ClientError, CoalesceOutcome, ControlAction, DraftRecord,
    DraftStore, HttpResponse, IdempotentActionQueue, InMemorySecretStore, MockTransport,
    OfflineStartupState, SandboxProvider, ScopedKeychainStore, SecretStore, ServerModeProvider,
    ThirdPartyAdapter,
};
use agents_client::ui::ClientController;
use agents_protocol::{CodecId, EventEnvelope, InteractionMode, StreamPayload, ThirdPartyProvider};

fn chunk_env(seq: u64, text: &str) -> EventEnvelope {
    EventEnvelope {
        seq,
        emitted_at_ms: 1_700_000_000_000 + seq,
        codec: CodecId::EventsV1,
        session_id: "sess-1".to_owned(),
        task_id: Some("task-1".to_owned()),
        payload: StreamPayload::TaskChunk {
            chunk: text.to_owned(),
            is_stderr: false,
        },
    }
}

#[test]
fn chunk_coalescer_reconciles_gaps_and_deduplicates_replays() {
    let mut coalescer = ChunkCoalescer::default();
    assert_eq!(
        coalescer.ingest(chunk_env(1, "hello ")),
        vec![CoalesceOutcome::Appended {
            seq: 1,
            chunk: Some("hello ".to_owned())
        }]
    );
    // Out-of-order seq 3 detects missing seq 2.
    assert_eq!(
        coalescer.ingest(chunk_env(3, "!")),
        vec![CoalesceOutcome::GapDetected {
            expected_seq: 2,
            received_seq: 3
        }]
    );
    // Replayed seq 1 ignored cleanly.
    assert_eq!(
        coalescer.ingest(chunk_env(1, "hello ")),
        vec![CoalesceOutcome::DuplicateIgnored { seq: 1 }]
    );
    // Reconnect replays seq 2, draining 2 and 3 in strict order.
    let drained = coalescer.ingest(chunk_env(2, "world"));
    assert_eq!(drained.len(), 2);
    assert_eq!(coalescer.last_applied_seq(), 3);
    assert_eq!(coalescer.merged_output(), "hello world!");
}

#[tokio::test]
async fn server_mode_provider_reconnect_replay_and_idempotent_controls() {
    let transport = Arc::new(MockTransport::with_responses(vec![
        HttpResponse {
            status: 200,
            body: r#"{"accessToken":"jwt-a","refreshToken":"ref-a","tokenType":"Bearer","expiresInSeconds":900}"#
                .to_owned(),
        },
        HttpResponse {
            status: 200,
            body: r#"{"id":"sess-42","title":"Fix bug","mode":"plan"}"#.to_owned(),
        },
        HttpResponse {
            status: 200,
            body: r#"{"id":"task-99"}"#.to_owned(),
        },
        HttpResponse {
            status: 200,
            body: r#"{"accepted":true}"#.to_owned(),
        },
        HttpResponse {
            status: 200,
            body: serde_json::to_string(&vec![chunk_env(2, "patched")]).unwrap(),
        },
    ]));

    let provider = Arc::new(ServerModeProvider::new(
        "https://agents.internal",
        transport.clone(),
    ));
    let secrets = Arc::new(InMemorySecretStore::default());
    let mut controller = ClientController::new(
        provider.clone(),
        secrets,
        BackendTarget::remote_server("https://agents.internal"),
    );

    controller
        .login_and_persist("leo@example.com", "secret-pass")
        .await
        .unwrap();
    let sess = controller
        .start_session("Fix bug", InteractionMode::Plan)
        .await
        .unwrap();
    assert_eq!(sess.id, "sess-42");

    let task = controller.submit_prompt("Inspect logs").await.unwrap();
    assert_eq!(task.task_id, "task-99");

    let receipt = controller
        .send_idempotent_control(&task.task_id, ControlAction::Pause)
        .await
        .unwrap();
    assert!(receipt.accepted);
    assert!(receipt.idempotency_key.contains("pause"));

    let replayed = provider.replay_events("jwt-a", &sess.id, 1).await.unwrap();
    assert_eq!(replayed.len(), 1);

    let reqs = transport.recorded_requests();
    assert_eq!(reqs.len(), 5);
    assert!(reqs[3].headers.contains_key("idempotency-key"));
    assert!(reqs[4].url.contains("after_seq=1"));
}

#[tokio::test]
async fn third_party_adapters_declare_honest_capabilities_and_fail_closed() {
    let transport = Arc::new(MockTransport::with_responses(vec![HttpResponse {
        status: 200,
        body: r#"{"id":"box-1"}"#.to_owned(),
    }]));
    let upstash = ThirdPartyAdapter::new(
        ThirdPartyProvider::UpstashBox,
        "https://box.upstash.io",
        transport.clone(),
    );
    let caps = upstash.capabilities();
    assert!(!caps.supports_fencing);
    assert!(!caps.supports_swarm);
    assert!(!caps.supports_live_steering);
    assert!(caps.supports_preview_routes);

    // Unsupported steer returns explicit UnsupportedCapability instead of silently faking.
    let err = upstash
        .steer_task("api-key", "exec-1", "change direction")
        .await
        .unwrap_err();
    assert!(matches!(
        err,
        ClientError::UnsupportedCapability {
            capability, ..
        } if capability == "live_steering"
    ));

    // Pause is unsupported on third-party adapters and fails closed.
    let pause_err = upstash
        .send_control("api-key", "exec-1", ControlAction::Pause, "k-1")
        .await
        .unwrap_err();
    assert!(matches!(
        pause_err,
        ClientError::UnsupportedCapability {
            capability, ..
        } if capability == "pause"
    ));

    // Replay is unsupported and fails closed.
    let replay_err = upstash
        .replay_events("api-key", "box-1", 0)
        .await
        .unwrap_err();
    assert!(matches!(
        replay_err,
        ClientError::UnsupportedCapability { .. }
    ));

    // Daytona honestly advertises live steering support.
    let daytona = ThirdPartyAdapter::new(
        ThirdPartyProvider::Daytona,
        "https://api.daytona.io",
        transport,
    );
    assert!(daytona.capabilities().supports_live_steering);
    assert!(!daytona.capabilities().supports_fencing);
}

#[test]
fn draft_recovery_offline_profile_queue_and_keychain_redaction_work() {
    let dir = std::env::temp_dir().join(format!("agents_keychain_test_{}", std::process::id()));
    let store = ScopedKeychainStore::new("natuleadan.agents.test", Some(dir.clone()));
    store
        .store_secret("jwt", "super-secret-token-value")
        .unwrap();
    let debug_str = format!("{store:?}");
    assert!(!debug_str.contains("super-secret-token-value"));
    assert_eq!(
        store.load_secret("jwt").unwrap().as_deref(),
        Some("super-secret-token-value")
    );
    store.delete_secret("jwt").unwrap();
    let _ = std::fs::remove_dir_all(dir);

    let drafts = DraftStore::default();
    drafts
        .save_startup_profile(OfflineStartupState {
            last_session_id: Some("sess-offline".to_owned()),
            ..OfflineStartupState::default()
        })
        .unwrap();
    drafts
        .save_draft(DraftRecord {
            session_id: "sess-offline".to_owned(),
            prompt: "unsent recovery prompt".to_owned(),
            attachments: vec!["diagram.png".to_owned()],
            updated_at_ms: 123,
        })
        .unwrap();

    let loaded = drafts.load_draft("sess-offline").unwrap().unwrap();
    assert_eq!(loaded.prompt, "unsent recovery prompt");

    let mut queue = IdempotentActionQueue::default();
    let a1 = queue.enqueue("t-1", ControlAction::Retry);
    assert_eq!(queue.pending().len(), 1);
    queue.acknowledge(&a1.idempotency_key);
    assert!(queue.pending().is_empty());
    assert!(queue.is_settled(&a1.idempotency_key));
}
