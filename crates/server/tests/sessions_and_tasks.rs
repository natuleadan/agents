#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use agents_protocol::{ExecutionScope, TaskState};
use agents_server::{ServerConfig, modules::tasks::repositories::TaskRepository};
use axum::http::{Method, StatusCode};
use serde_json::json;

use common::{build_test_harness, register_and_login, send_json, send_json_with_headers};

#[tokio::test]
async fn sessions_bola_tristate_filter_pagination_optimistic_lock_and_drafts() {
    let harness = build_test_harness(ServerConfig::for_tests()).await;
    let (_, alice_jwt) =
        register_and_login(&harness.router, "alice@example.com", "password-alice-123").await;
    let (_, bob_jwt) =
        register_and_login(&harness.router, "bob@example.com", "password-bob-1234").await;

    // Create 3 sessions for Alice, archiving the second one.
    let mut session_ids = Vec::new();
    for idx in 0..3 {
        let (st, body) = send_json(
            &harness.router,
            Method::POST,
            "/api/v1/sessions",
            Some(&alice_jwt),
            Some(json!({ "title": format!("Session {idx}"), "mode": "plan" })),
        )
        .await;
        assert_eq!(st, StatusCode::OK);
        session_ids.push(body["id"].as_str().unwrap().to_owned());
    }

    // Archive session 1 with expectedVersion = 1.
    let (arch_st, arch_body) = send_json(
        &harness.router,
        Method::PATCH,
        &format!("/api/v1/sessions/{}", session_ids[1]),
        Some(&alice_jwt),
        Some(json!({ "archived": true, "expectedVersion": 1 })),
    )
    .await;
    assert_eq!(arch_st, StatusCode::OK);
    assert_eq!(arch_body["version"], 2);
    assert!(arch_body["archivedAt"].is_number());

    // Optimistic concurrency conflict with stale expectedVersion = 1 returns 409.
    let (conflict_st, _) = send_json(
        &harness.router,
        Method::PATCH,
        &format!("/api/v1/sessions/{}", session_ids[1]),
        Some(&alice_jwt),
        Some(json!({ "title": "Stale write", "expectedVersion": 1 })),
    )
    .await;
    assert_eq!(conflict_st, StatusCode::CONFLICT);

    // Tri-state filters: archived=true -> 1, archived=null -> 2, omitted -> 3.
    let (_, only_archived) = send_json(
        &harness.router,
        Method::GET,
        "/api/v1/sessions?archived=true",
        Some(&alice_jwt),
        None,
    )
    .await;
    assert_eq!(only_archived["items"].as_array().unwrap().len(), 1);

    let (_, unarchived) = send_json(
        &harness.router,
        Method::GET,
        "/api/v1/sessions?archived=null",
        Some(&alice_jwt),
        None,
    )
    .await;
    assert_eq!(unarchived["items"].as_array().unwrap().len(), 2);

    // Cursor pagination (limit=2) returns nextCursor for the third item.
    let (_, page_1) = send_json(
        &harness.router,
        Method::GET,
        "/api/v1/sessions?limit=2",
        Some(&alice_jwt),
        None,
    )
    .await;
    assert_eq!(page_1["items"].as_array().unwrap().len(), 2);
    let cursor = page_1["nextCursor"].as_str().unwrap();

    let (_, page_2) = send_json(
        &harness.router,
        Method::GET,
        &format!("/api/v1/sessions?limit=2&cursor={cursor}"),
        Some(&alice_jwt),
        None,
    )
    .await;
    assert_eq!(page_2["items"].as_array().unwrap().len(), 1);

    // Draft persistence & recovery.
    let (draft_put_st, _) = send_json(
        &harness.router,
        Method::PUT,
        &format!("/api/v1/sessions/{}/draft", session_ids[0]),
        Some(&alice_jwt),
        Some(json!({ "content": "unsent recovery prompt" })),
    )
    .await;
    assert_eq!(draft_put_st, StatusCode::OK);

    let (draft_get_st, draft_body) = send_json(
        &harness.router,
        Method::GET,
        &format!("/api/v1/sessions/{}/draft", session_ids[0]),
        Some(&alice_jwt),
        None,
    )
    .await;
    assert_eq!(draft_get_st, StatusCode::OK);
    assert_eq!(draft_body["content"], "unsent recovery prompt");

    // BOLA: Bob gets 403 Forbidden on Alice's session, patch, draft, and delete.
    for (method, path, payload) in [
        (
            Method::GET,
            format!("/api/v1/sessions/{}", session_ids[0]),
            None,
        ),
        (
            Method::PATCH,
            format!("/api/v1/sessions/{}", session_ids[0]),
            Some(json!({ "title": "Hijacked" })),
        ),
        (
            Method::GET,
            format!("/api/v1/sessions/{}/draft", session_ids[0]),
            None,
        ),
        (
            Method::DELETE,
            format!("/api/v1/sessions/{}", session_ids[0]),
            None,
        ),
    ] {
        let (st, _) = send_json(&harness.router, method, &path, Some(&bob_jwt), payload).await;
        assert_eq!(st, StatusCode::FORBIDDEN);
    }
}

#[tokio::test]
async fn tasks_idempotency_fencing_tokens_steering_and_admission_backpressure() {
    let harness = build_test_harness(ServerConfig::for_tests()).await;
    let (_, alice_jwt) =
        register_and_login(&harness.router, "alice@example.com", "password-alice-123").await;
    let (_, bob_jwt) =
        register_and_login(&harness.router, "bob@example.com", "password-bob-1234").await;

    let (_, session) = send_json(
        &harness.router,
        Method::POST,
        "/api/v1/sessions",
        Some(&alice_jwt),
        Some(json!({ "title": "Work session" })),
    )
    .await;
    let sid = session["id"].as_str().unwrap().to_owned();

    // Duplicate Idempotency-Key returns the same task and publishes once to NATS bus.
    let (st1, task1) = send_json_with_headers(
        &harness.router,
        Method::POST,
        &format!("/api/v1/sessions/{sid}/tasks"),
        Some(&alice_jwt),
        &[("idempotency-key", "req-001")],
        Some(json!({ "prompt": "Build module" })),
    )
    .await;
    assert_eq!(st1, StatusCode::OK);
    let task_id = task1["id"].as_str().unwrap().to_owned();

    let (st2, task2) = send_json_with_headers(
        &harness.router,
        Method::POST,
        &format!("/api/v1/sessions/{sid}/tasks"),
        Some(&alice_jwt),
        &[("idempotency-key", "req-001")],
        Some(json!({ "prompt": "Build module" })),
    )
    .await;
    assert_eq!(st2, StatusCode::OK);
    assert_eq!(task1["id"], task2["id"]);
    assert_eq!(harness.bus.recorded_task_requests().await.len(), 1);

    // BOLA check: Bob cannot read or steer Alice's task.
    let (bob_get, _) = send_json(
        &harness.router,
        Method::GET,
        &format!("/api/v1/tasks/{task_id}"),
        Some(&bob_jwt),
        None,
    )
    .await;
    assert_eq!(bob_get, StatusCode::FORBIDDEN);

    // Boxer worker leases task -> fencingToken = 1.
    let (lease_st, lease) = send_json(
        &harness.router,
        Method::POST,
        &format!("/api/v1/tasks/{task_id}/lease"),
        Some(&alice_jwt),
        Some(json!({ "boxerNodeId": "boxer-miami-1", "leaseTtlSeconds": 300 })),
    )
    .await;
    assert_eq!(lease_st, StatusCode::OK);
    assert_eq!(lease["fencingToken"], 1);

    // Second concurrent lease fails with 409 Conflict.
    let (lease_dup, _) = send_json(
        &harness.router,
        Method::POST,
        &format!("/api/v1/tasks/{task_id}/lease"),
        Some(&alice_jwt),
        Some(json!({ "boxerNodeId": "boxer-miami-2" })),
    )
    .await;
    assert_eq!(lease_dup, StatusCode::CONFLICT);

    // Transition with valid fencing token 1 -> Running; stale token 99 -> 409 Conflict.
    let (run_st, run_task) = send_json(
        &harness.router,
        Method::POST,
        &format!("/api/v1/tasks/{task_id}/state"),
        Some(&alice_jwt),
        Some(json!({ "fencingToken": 1, "state": TaskState::Running })),
    )
    .await;
    assert_eq!(run_st, StatusCode::OK);
    assert_eq!(run_task["state"], "running");

    let (stale_st, _) = send_json(
        &harness.router,
        Method::POST,
        &format!("/api/v1/tasks/{task_id}/state"),
        Some(&alice_jwt),
        Some(json!({ "fencingToken": 99, "state": TaskState::Blocked })),
    )
    .await;
    assert_eq!(stale_st, StatusCode::CONFLICT);

    // Steer and follow-up work while task is active.
    let (steer_st, _) = send_json(
        &harness.router,
        Method::POST,
        &format!("/api/v1/tasks/{task_id}/steer"),
        Some(&alice_jwt),
        Some(json!({ "content": "Prefer async traits" })),
    )
    .await;
    assert_eq!(steer_st, StatusCode::OK);
    assert_eq!(harness.bus.recorded_steering().await.len(), 1);

    let (fu_st, _) = send_json(
        &harness.router,
        Method::POST,
        &format!("/api/v1/sessions/{sid}/follow-ups"),
        Some(&alice_jwt),
        Some(json!({ "content": "Run validation afterward" })),
    )
    .await;
    assert_eq!(fu_st, StatusCode::OK);

    // Complete task cleanly with fencingToken = 1.
    let (done_st, done_task) = send_json(
        &harness.router,
        Method::POST,
        &format!("/api/v1/tasks/{task_id}/complete"),
        Some(&alice_jwt),
        Some(json!({
            "fencingToken": 1,
            "exitCode": 0,
            "stopReason": "end_turn",
            "outputSummary": "Compiled cleanly"
        })),
    )
    .await;
    assert_eq!(done_st, StatusCode::OK);
    assert_eq!(done_task["state"], "done");

    // Admission backpressure: saturate per-session capacity -> 503 Overloaded, release on Drop.
    let p1 = harness
        .admission
        .try_acquire(ExecutionScope::Session, Some(&sid))
        .unwrap();
    let p2 = harness
        .admission
        .try_acquire(ExecutionScope::Session, Some(&sid))
        .unwrap();

    let (overloaded_st, _) = send_json(
        &harness.router,
        Method::POST,
        &format!("/api/v1/sessions/{sid}/tasks"),
        Some(&alice_jwt),
        Some(json!({ "prompt": "Should hit admission backpressure" })),
    )
    .await;
    assert_eq!(overloaded_st, StatusCode::SERVICE_UNAVAILABLE);

    drop(p1);
    drop(p2);
    let (recovered_st, _) = send_json(
        &harness.router,
        Method::POST,
        &format!("/api/v1/sessions/{sid}/tasks"),
        Some(&alice_jwt),
        Some(json!({ "prompt": "Admitted after drop" })),
    )
    .await;
    assert_eq!(recovered_st, StatusCode::OK);

    // leaseTtlSeconds = 0 is rejected by garde (422), while an expired lease allows failover re-lease (fencingToken = 2).
    let (_, exp_task) = send_json(
        &harness.router,
        Method::POST,
        &format!("/api/v1/sessions/{sid}/tasks"),
        Some(&alice_jwt),
        Some(json!({ "prompt": "Failover lease test" })),
    )
    .await;
    let exp_id = exp_task["id"].as_str().unwrap();

    let (invalid_ttl_st, _) = send_json(
        &harness.router,
        Method::POST,
        &format!("/api/v1/tasks/{exp_id}/lease"),
        Some(&alice_jwt),
        Some(json!({ "boxerNodeId": "boxer-old", "leaseTtlSeconds": 0 })),
    )
    .await;
    assert_eq!(invalid_ttl_st, StatusCode::UNPROCESSABLE_ENTITY);

    let l1 = harness
        .task_repo
        .acquire_lease(exp_id, "boxer-old", 100, 200)
        .await
        .unwrap();
    assert_eq!(l1.fencing_token.0, 1);

    let (l2_st, l2) = send_json(
        &harness.router,
        Method::POST,
        &format!("/api/v1/tasks/{exp_id}/lease"),
        Some(&alice_jwt),
        Some(json!({ "boxerNodeId": "boxer-new", "leaseTtlSeconds": 60 })),
    )
    .await;
    assert_eq!(l2_st, StatusCode::OK);
    assert_eq!(l2["fencingToken"], 2);
}
