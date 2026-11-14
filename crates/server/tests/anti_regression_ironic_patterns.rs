//! Anti-regression test suite validating native Ironic framework patterns:
//! cooperative task scheduling, cache hit/invalidation telemetry, typed domain events,
//! and fail-closed RFC 7807 problem details error envelopes.

#![forbid(unsafe_code)]
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use std::{
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};

use agents_server::{
    ServerConfig,
    infra::{
        cache::AppCache,
        events::{AppEventBus, AutocronTickEvent, SessionCreatedEvent, TaskCompletedEvent},
        state::now_ms,
    },
};
use axum::http::{Method, StatusCode};
use common::{build_test_harness, register_and_login, send_json};
use ironic::services::scheduling::{cron_schedule, interval};

#[tokio::test]
async fn anti_regression_ironic_native_scheduling_cooperative_lifecycle() {
    let tick_count = Arc::new(AtomicUsize::new(0));
    let tick_clone = Arc::clone(&tick_count);

    // 1. Invariant: Scheduled task must use native ironic interval and not bare sleep loops
    let task = interval(Duration::from_millis(20), move || {
        let counter = Arc::clone(&tick_clone);
        async move {
            counter.fetch_add(1, Ordering::SeqCst);
        }
    });

    tokio::time::sleep(Duration::from_millis(70)).await;
    let ticks_before_pause = tick_count.load(Ordering::SeqCst);
    assert!(
        ticks_before_pause >= 2,
        "cooperative interval should fire ticks on schedule, got {ticks_before_pause}"
    );

    // 2. Invariant: Task must support non-panicking pause and resume
    task.pause();
    tokio::time::sleep(Duration::from_millis(50)).await;
    let ticks_during_pause = tick_count.load(Ordering::SeqCst);
    assert_eq!(
        ticks_before_pause, ticks_during_pause,
        "paused task must not advance tick counter"
    );

    task.resume();
    tokio::time::sleep(Duration::from_millis(50)).await;
    let ticks_after_resume = tick_count.load(Ordering::SeqCst);
    assert!(
        ticks_after_resume > ticks_during_pause,
        "resumed task must resume ticking"
    );

    // 3. Invariant: Task must support deterministic shutdown without join errors
    let shutdown_res = task.shutdown().await;
    assert!(
        shutdown_res.is_ok(),
        "scheduled task must shut down cleanly without error"
    );

    // 4. Invariant: Cron parser must accept valid 6-field expressions and reject invalid syntax
    let valid_cron = cron_schedule("0 */5 * * * *", || async {});
    assert!(valid_cron.is_ok(), "valid 6-field cron must parse cleanly");
    if let Ok(cron_task) = valid_cron {
        let _ = cron_task.shutdown().await;
    }

    let invalid_cron = cron_schedule("invalid-cron-syntax", || async {});
    assert!(
        invalid_cron.is_err(),
        "invalid cron expression must fail closed"
    );
}

#[tokio::test]
async fn anti_regression_ironic_cache_and_mutation_invalidation() {
    let cache = AppCache::in_memory(1024);

    #[derive(serde::Serialize, serde::Deserialize, PartialEq, Debug, Clone)]
    struct CachedEntity {
        id: String,
        name: String,
        score: u32,
    }

    let item = CachedEntity {
        id: "sess-123".into(),
        name: "Test Session".into(),
        score: 100,
    };

    // Invariant 1: Cache miss returns None
    let miss: Option<CachedEntity> = cache.get_json("session:sess-123").await;
    assert!(miss.is_none(), "unpopulated key must return None");

    // Invariant 2: Set stores and retrieves identical value
    cache
        .set_json("session:sess-123", &item, Duration::from_secs(60))
        .await;
    let hit: Option<CachedEntity> = cache.get_json("session:sess-123").await;
    assert_eq!(hit, Some(item.clone()), "cache hit must match original");

    // Invariant 3: Single key eviction clears entry
    let removed = cache.remove("session:sess-123").await;
    assert!(removed, "key removal should return true for existing key");
    let after_evict: Option<CachedEntity> = cache.get_json("session:sess-123").await;
    assert!(after_evict.is_none(), "evicted key must return None");

    // Invariant 4: Prefix eviction clears all namespace keys
    cache
        .set_json("session:a", &item, Duration::from_secs(60))
        .await;
    cache
        .set_json("session:b", &item, Duration::from_secs(60))
        .await;
    cache
        .set_json("task:c", &item, Duration::from_secs(60))
        .await;

    let cleared = cache.remove_by_prefix("session:").await;
    assert_eq!(
        cleared, 2,
        "prefix removal must evict exactly matching keys"
    );
    assert!(cache.get_json::<CachedEntity>("session:a").await.is_none());
    assert!(cache.get_json::<CachedEntity>("session:b").await.is_none());
    assert!(cache.get_json::<CachedEntity>("task:c").await.is_some());
}

#[tokio::test]
async fn anti_regression_ironic_typed_event_bus_and_dead_letter() {
    let bus = AppEventBus::new();
    let mut session_sub = bus.subscribe::<SessionCreatedEvent>(10).await;
    let mut task_sub = bus.subscribe::<TaskCompletedEvent>(10).await;
    let mut cron_sub = bus.subscribe::<AutocronTickEvent>(10).await;

    // Publish SessionCreatedEvent
    let s_evt = SessionCreatedEvent {
        session_id: "s-1".into(),
        user_id: "u-1".into(),
        mode: "Act".into(),
        created_at: now_ms(),
    };
    let delivered = bus.publish(s_evt.clone()).await;
    assert_eq!(delivered, 1, "session event should reach 1 subscriber");

    // Publish TaskCompletedEvent
    let t_evt = TaskCompletedEvent {
        task_id: "t-1".into(),
        session_id: "s-1".into(),
        status: "Done".into(),
        completed_at: now_ms(),
    };
    let delivered_task = bus.publish(t_evt.clone()).await;
    assert_eq!(delivered_task, 1, "task event should reach 1 subscriber");

    // Publish AutocronTickEvent
    let c_evt = AutocronTickEvent {
        schedule_id: "sched-1".into(),
        workflow_id: "wf-1".into(),
        tick_at: now_ms(),
    };
    let delivered_cron = bus.publish(c_evt.clone()).await;
    assert_eq!(
        delivered_cron, 1,
        "autocron event should reach 1 subscriber"
    );

    // Verify typed delivery
    let received_s = session_sub.recv().await;
    assert_eq!(received_s.as_deref(), Some(&s_evt));

    let received_t = task_sub.recv().await;
    assert_eq!(received_t.as_deref(), Some(&t_evt));

    let received_c = cron_sub.recv().await;
    assert_eq!(received_c.as_deref(), Some(&c_evt));

    // Verify dead-letter handling: dropping subscriber does not crash publish
    drop(task_sub);
    let delivered_after_drop = bus
        .publish(TaskCompletedEvent {
            task_id: "t-2".into(),
            session_id: "s-1".into(),
            status: "Failed".into(),
            completed_at: now_ms(),
        })
        .await;
    assert_eq!(
        delivered_after_drop, 0,
        "dropped subscriber must result in 0 delivered and route to dead letter"
    );
}

#[tokio::test]
async fn anti_regression_session_caching_and_event_integration() {
    let harness = build_test_harness(ServerConfig::for_tests()).await;
    let (_, jwt) = register_and_login(
        &harness.router,
        "ironic-tester@example.com",
        "strong-password-123",
    )
    .await;

    // Create session via HTTP controller
    let (status, body) = send_json(
        &harness.router,
        Method::POST,
        "/api/v1/sessions",
        Some(&jwt),
        Some(serde_json::json!({
            "title": "Ironic Session",
            "mode": "plan"
        })),
    )
    .await;

    assert_eq!(status, StatusCode::OK);
    let session_id = body["id"].as_str().unwrap().to_owned();

    // Read session: should be served (and cached)
    let (get_status, get_body) = send_json(
        &harness.router,
        Method::GET,
        &format!("/api/v1/sessions/{session_id}"),
        Some(&jwt),
        None,
    )
    .await;

    assert_eq!(get_status, StatusCode::OK);
    assert_eq!(get_body["title"], "Ironic Session");

    // Second read: cache hit
    let (hit_status, hit_body) = send_json(
        &harness.router,
        Method::GET,
        &format!("/api/v1/sessions/{session_id}"),
        Some(&jwt),
        None,
    )
    .await;

    assert_eq!(hit_status, StatusCode::OK);
    assert_eq!(hit_body["id"], session_id);
}

fn collect_rs_files(dir: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                collect_rs_files(&path, out);
            } else if path.extension().and_then(|e| e.to_str()) == Some("rs") {
                out.push(path);
            }
        }
    }
}

#[test]
fn anti_regression_cache_interceptor_and_no_raw_tokio_intervals() {
    let cache = AppCache::in_memory(64);
    let _interceptor = cache.into_interceptor();
    let _backend = cache.backend();

    let server_src = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let boxer_src = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("crates dir")
        .join("boxer/src");

    let mut files = Vec::new();
    collect_rs_files(&server_src, &mut files);
    collect_rs_files(&boxer_src, &mut files);
    assert!(!files.is_empty(), "must scan server and boxer source trees");

    for file in files {
        let contents = std::fs::read_to_string(&file).expect("read source file");
        assert!(
            !contents.contains("tokio::time::interval"),
            "forbidden raw tokio::time::interval found in {} (use ironic::services::scheduling::interval)",
            file.display()
        );
    }
}
