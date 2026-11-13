#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use agents_protocol::{PermissionMask, TriggerKind, WorkflowNode};
use agents_server::{
    ServerConfig,
    modules::{
        autocron::services::{
            AutocronService,
            cron::{CronSpec, compute_next_run_ms, parse_timezone_offset},
        },
        workflows::{
            entities::{WorkflowEntity, WorkflowRunEntity, WorkflowRunStatus},
            repositories::WorkflowRepository,
        },
    },
};
use axum::http::{Method, StatusCode};
use serde_json::json;

use common::{build_test_harness, register_and_login, send_json};

fn full_mask() -> PermissionMask {
    PermissionMask {
        can_read_fs: true,
        can_write_fs: true,
        can_exec_shell: true,
        can_network: true,
        allowed_tools: vec!["grep".to_owned(), "read".to_owned()],
    }
}

fn read_only_mask() -> PermissionMask {
    PermissionMask {
        can_read_fs: true,
        can_write_fs: false,
        can_exec_shell: false,
        can_network: false,
        allowed_tools: vec!["grep".to_owned(), "read".to_owned()],
    }
}

#[tokio::test]
async fn swarm_http_asi07_narrowing_asi05_signature_bola_and_valkey_blackboard() {
    let harness = build_test_harness(ServerConfig::for_tests()).await;
    let (_, alice_jwt) =
        register_and_login(&harness.router, "alice-swarm@example.com", "pw-alice-1234").await;
    let (_, bob_jwt) =
        register_and_login(&harness.router, "bob-swarm@example.com", "pw-bob-123456").await;

    let (sess_st, sess_body) = send_json(
        &harness.router,
        Method::POST,
        "/api/v1/sessions",
        Some(&alice_jwt),
        Some(json!({ "title": "Swarm coordination", "mode": "act" })),
    )
    .await;
    assert_eq!(sess_st, StatusCode::OK);
    let session_id = sess_body["id"].as_str().unwrap();

    // BOLA check: Bob cannot handoff or read Alice's swarm blackboard.
    let (bola_st, _) = send_json(
        &harness.router,
        Method::GET,
        &format!("/api/v1/sessions/{session_id}/swarm/handoffs"),
        Some(&bob_jwt),
        None,
    )
    .await;
    assert_eq!(bola_st, StatusCode::FORBIDDEN);

    // OWASP ASI-07: child escalation beyond parent permissions is rejected fail-closed.
    let parent_mask = read_only_mask();
    let mut escalated_mask = read_only_mask();
    escalated_mask.can_exec_shell = true;

    let (esc_st, esc_body) = send_json(
        &harness.router,
        Method::POST,
        &format!("/api/v1/sessions/{session_id}/swarm/handoffs"),
        Some(&alice_jwt),
        Some(json!({
            "parentAgentId": "lead-agent",
            "childAgentId": "worker-agent",
            "parentMask": parent_mask,
            "requestedMask": escalated_mask,
            "goal": "escalate privileges",
        })),
    )
    .await;
    assert_eq!(esc_st, StatusCode::FORBIDDEN, "body={esc_body}");

    // Valid narrowed delegation succeeds.
    let (ok_st, ok_body) = send_json(
        &harness.router,
        Method::POST,
        &format!("/api/v1/sessions/{session_id}/swarm/handoffs"),
        Some(&alice_jwt),
        Some(json!({
            "parentAgentId": "lead-agent",
            "childAgentId": "reviewer-agent",
            "parentMask": full_mask(),
            "requestedMask": read_only_mask(),
            "worktreeBranch": "feat/review-subtask",
            "goal": "review diffs read-only",
        })),
    )
    .await;
    assert_eq!(ok_st, StatusCode::OK, "body={ok_body}");
    assert_eq!(ok_body["delegatedMask"]["canExecShell"], false);

    // Valkey shared blackboard write & read roundtrip.
    let (bb_put_st, _) = send_json(
        &harness.router,
        Method::POST,
        &format!("/api/v1/sessions/{session_id}/swarm/blackboard"),
        Some(&alice_jwt),
        Some(json!({
            "key": "design_notes",
            "value": { "step": "dag-verified", "owners": ["lead", "reviewer"] }
        })),
    )
    .await;
    assert_eq!(bb_put_st, StatusCode::OK);

    let (bb_get_st, bb_get) = send_json(
        &harness.router,
        Method::GET,
        &format!("/api/v1/sessions/{session_id}/swarm/blackboard/design_notes"),
        Some(&alice_jwt),
        None,
    )
    .await;
    assert_eq!(bb_get_st, StatusCode::OK);
    assert_eq!(bb_get["value"]["step"], "dag-verified");

    // OWASP ASI-05: forged HMAC signature on swarm message fails closed.
    let (bad_sig_st, _) = send_json(
        &harness.router,
        Method::POST,
        &format!("/api/v1/sessions/{session_id}/swarm/messages"),
        Some(&alice_jwt),
        Some(json!({
            "fromAgentId": "lead-agent",
            "targetAgentId": "reviewer-agent",
            "payload": { "instruction": "audit crate" },
            "permissionMask": read_only_mask(),
            "signature": "forged-deadbeef-signature",
        })),
    )
    .await;
    assert_eq!(bad_sig_st, StatusCode::FORBIDDEN);

    // Server-signed mediated message succeeds and records `mediatedByUser`.
    let (good_msg_st, good_msg) = send_json(
        &harness.router,
        Method::POST,
        &format!("/api/v1/sessions/{session_id}/swarm/messages"),
        Some(&alice_jwt),
        Some(json!({
            "fromAgentId": "lead-agent",
            "targetAgentId": "reviewer-agent",
            "payload": { "instruction": "audit crate" },
            "permissionMask": read_only_mask(),
        })),
    )
    .await;
    assert_eq!(good_msg_st, StatusCode::OK);
    assert!(good_msg["mediatedByUser"].is_string());
    assert!(!good_msg["signature"].as_str().unwrap().is_empty());
}

#[tokio::test]
async fn workflows_reject_cycles_evaluate_conditional_edges_and_persist_checkpoints() {
    let harness = build_test_harness(ServerConfig::for_tests()).await;
    let (_, alice_jwt) =
        register_and_login(&harness.router, "alice-wf@example.com", "pw-alice-wf-12").await;

    // Cyclic workflow must be rejected with 422.
    let (cyc_st, _) = send_json(
        &harness.router,
        Method::POST,
        "/api/v1/workflows",
        Some(&alice_jwt),
        Some(json!({
            "name": "Cyclic DAG",
            "nodes": [
                { "nodeId": "n1", "kind": "condition", "config": {} },
                { "nodeId": "n2", "kind": "agent_step", "config": {} },
            ],
            "edges": [
                { "fromNodeId": "n1", "toNodeId": "n2" },
                { "fromNodeId": "n2", "toNodeId": "n1" },
            ]
        })),
    )
    .await;
    assert_eq!(cyc_st, StatusCode::UNPROCESSABLE_ENTITY);

    // Valid conditional DAG executes active branch and skips inactive branch.
    let (wf_st, wf_body) = send_json(
        &harness.router,
        Method::POST,
        "/api/v1/workflows",
        Some(&alice_jwt),
        Some(json!({
            "name": "Branching Pipeline",
            "nodes": [
                { "nodeId": "check", "kind": "condition", "config": { "branch": true } },
                { "nodeId": "deploy_step", "kind": "agent_step", "config": { "prompt": "deploy preview" } },
                { "nodeId": "skip_step", "kind": "agent_step", "config": { "prompt": "should skip" } }
            ],
            "edges": [
                { "fromNodeId": "check", "toNodeId": "deploy_step", "condition": "branch == true" },
                { "fromNodeId": "check", "toNodeId": "skip_step", "condition": "branch == false" }
            ]
        })),
    )
    .await;
    assert_eq!(wf_st, StatusCode::OK);
    let wf_id = wf_body["workflowId"].as_str().unwrap();

    let (run_st, run_body) = send_json(
        &harness.router,
        Method::POST,
        &format!("/api/v1/workflows/{wf_id}/trigger"),
        Some(&alice_jwt),
        Some(json!({ "input": { "env": "staging" } })),
    )
    .await;
    assert_eq!(run_st, StatusCode::OK, "body={run_body}");
    assert_eq!(run_body["status"], "completed");

    let completed = run_body["checkpointState"]["completedNodes"]
        .as_array()
        .unwrap();
    let completed_names: Vec<&str> = completed.iter().filter_map(|v| v.as_str()).collect();
    assert!(completed_names.contains(&"check"));
    assert!(completed_names.contains(&"deploy_step"));
    assert!(!completed_names.contains(&"skip_step"));

    let dispatched = harness.bus.recorded_task_requests().await;
    assert_eq!(dispatched.len(), 1);
    assert_eq!(dispatched[0].prompt, "deploy preview");
}

#[tokio::test]
async fn autocron_timezone_budget_cap_webhook_ambient_and_boot_id_crash_recovery() {
    let harness = build_test_harness(ServerConfig::for_tests()).await;
    let (alice_id, alice_jwt) =
        register_and_login(&harness.router, "alice-cron@example.com", "pw-alice-cron-1").await;

    // Verify timezone offsets and cron scheduling differences across timezones.
    let bogota_tz = parse_timezone_offset("America/Bogota").unwrap();
    let madrid_tz = parse_timezone_offset("Europe/Madrid").unwrap();
    let spec = CronSpec::parse("30 9 * * *").unwrap();
    let base_ms = 1_700_000_000_000_u64;
    let next_bogota = spec.next_after_ms(base_ms, bogota_tz).unwrap();
    let next_madrid = spec.next_after_ms(base_ms, madrid_tz).unwrap();
    assert_ne!(next_bogota, next_madrid);
    assert!(
        compute_next_run_ms(
            &TriggerKind::Cron,
            Some("*/5 * * * *"),
            None,
            "America/Bogota",
            base_ms
        )
        .unwrap()
        .is_some()
    );

    // Create simple workflow for schedule tests.
    let (_, wf_body) = send_json(
        &harness.router,
        Method::POST,
        "/api/v1/workflows",
        Some(&alice_jwt),
        Some(json!({
            "name": "Cron Target",
            "nodes": [{ "nodeId": "step1", "kind": "agent_step", "config": { "prompt": "nightly check" } }],
            "edges": []
        })),
    )
    .await;
    let wf_id = wf_body["workflowId"].as_str().unwrap();

    // Invalid timezone fails validation cleanly.
    let (bad_tz_st, _) = send_json(
        &harness.router,
        Method::POST,
        "/api/v1/autocron/schedules",
        Some(&alice_jwt),
        Some(json!({
            "workflowId": wf_id,
            "trigger": "cron",
            "cronExpr": "0 * * * *",
            "timezone": "Mars/Olympus_Mons"
        })),
    )
    .await;
    assert_eq!(bad_tz_st, StatusCode::UNPROCESSABLE_ENTITY);

    // Budget-capped ambient schedule in America/Bogota.
    let (sched_st, sched_body) = send_json(
        &harness.router,
        Method::POST,
        "/api/v1/autocron/schedules",
        Some(&alice_jwt),
        Some(json!({
            "workflowId": wf_id,
            "trigger": "interval_seconds",
            "intervalSeconds": 300,
            "timezone": "America/Bogota",
            "maxBudgetUsdCents": 100,
            "ambientMode": true
        })),
    )
    .await;
    assert_eq!(sched_st, StatusCode::OK);
    let sched_id = sched_body["scheduleId"].as_str().unwrap();

    // First run (60 cents) succeeds with ambient notification and 40 cents remaining.
    let (r1_st, r1_body) = send_json(
        &harness.router,
        Method::POST,
        &format!("/api/v1/autocron/schedules/{sched_id}/run"),
        Some(&alice_jwt),
        Some(json!({ "estimatedCostCents": 60 })),
    )
    .await;
    assert_eq!(r1_st, StatusCode::OK, "body={r1_body}");
    assert_eq!(r1_body["ambientDispatched"], true);
    assert_eq!(r1_body["remainingBudgetCents"], 40);

    // Second run exceeding budget ceiling (60 > 40 remaining) returns 409 and disables schedule.
    let (r2_st, _) = send_json(
        &harness.router,
        Method::POST,
        &format!("/api/v1/autocron/schedules/{sched_id}/run"),
        Some(&alice_jwt),
        Some(json!({ "estimatedCostCents": 60 })),
    )
    .await;
    assert_eq!(r2_st, StatusCode::CONFLICT);

    // Webhook-triggered schedule executes via webhook endpoint.
    let (wh_sched_st, wh_sched) = send_json(
        &harness.router,
        Method::POST,
        "/api/v1/autocron/schedules",
        Some(&alice_jwt),
        Some(json!({
            "workflowId": wf_id,
            "trigger": "webhook",
            "timezone": "UTC",
        })),
    )
    .await;
    assert_eq!(wh_sched_st, StatusCode::OK);
    let wh_id = wh_sched["scheduleId"].as_str().unwrap();

    let (wh_run_st, wh_run) = send_json(
        &harness.router,
        Method::POST,
        &format!("/api/v1/autocron/webhooks/{wh_id}"),
        Some(&alice_jwt),
        Some(json!({ "payload": { "event": "push" } })),
    )
    .await;
    assert_eq!(wh_run_st, StatusCode::OK);
    assert!(wh_run["runId"].is_string());

    // Crash-resume recovery across `boot_id` restart.
    let recover_wf = WorkflowEntity {
        workflow_id: "wf-crash-recover".to_owned(),
        owner_id: alice_id.clone(),
        name: "Crash Resume DAG".to_owned(),
        nodes: vec![WorkflowNode {
            node_id: "resume_node".to_owned(),
            kind: "agent_step".to_owned(),
            config: json!({ "prompt": "finish recovered step" }),
        }],
        edges: Vec::new(),
        created_at: 10,
        updated_at: 10,
    };
    harness
        .workflow_repo
        .create_workflow(recover_wf)
        .await
        .unwrap();

    let crashed_run = WorkflowRunEntity {
        run_id: "run-orphaned-1".to_owned(),
        workflow_id: "wf-crash-recover".to_owned(),
        schedule_id: None,
        owner_id: alice_id,
        boot_id: "old-crashed-boot".to_owned(),
        status: WorkflowRunStatus::Running,
        current_node_id: Some("resume_node".to_owned()),
        checkpoint_state: json!({ "input": {}, "completedNodes": [], "outputs": {} }),
        resumed_count: 0,
        started_at: 20,
        completed_at: None,
    };
    harness.workflow_repo.upsert_run(crashed_run).await.unwrap();

    let autocron = harness
        .app
        .container()
        .resolve::<AutocronService>()
        .await
        .unwrap();
    let report = autocron.recover_on_boot("fresh-boot-2").await.unwrap();
    assert_eq!(report.resumed_runs, 1);

    let recovered = harness
        .workflow_repo
        .find_run("run-orphaned-1")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(recovered.status, WorkflowRunStatus::Completed);
    assert_eq!(recovered.boot_id, "fresh-boot-2");
    assert_eq!(recovered.resumed_count, 1);
}
