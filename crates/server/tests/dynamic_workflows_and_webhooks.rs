#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use agents_server::ServerConfig;
use axum::http::{Method, StatusCode};
use common::{build_test_harness, register_and_login, send_json};
use serde_json::json;

#[tokio::test]
async fn agent_created_workflow_requires_approval_before_webhook_and_render() {
    let harness = build_test_harness(ServerConfig::for_tests()).await;
    let (_user_id, token) =
        register_and_login(&harness.router, "supervisor@domain.test", "P@ssword123456!").await;

    // 1. Create workflow tagged as created by agent
    let (create_st, create_res) = send_json(
        &harness.router,
        Method::POST,
        "/api/v1/workflows",
        Some(&token),
        Some(json!({
            "name": "Autonomous Poem Publisher",
            "webhookSlug": "poem-bot",
            "renderSlug": "poem-bot",
            "createdByAgent": true,
            "nodes": [
                {
                    "nodeId": "step_1",
                    "kind": "render_html",
                    "config": {
                        "html": "<html><body><h1>The Silent Night</h1><p>Moonlit shadows dance...</p></body></html>"
                    }
                }
            ],
            "edges": []
        })),
    )
    .await;

    assert_eq!(create_st, StatusCode::OK, "create failed: {create_res}");
    let workflow_id = create_res["workflowId"].as_str().unwrap();
    assert_eq!(
        create_res["approved"], false,
        "agent workflows must default to pending approval (OWASP ASI-02)"
    );

    // 2. Call dynamic webhook BEFORE approval -> must be rejected 403 Forbidden
    let (wh_pre_st, _wh_pre_res) = send_json(
        &harness.router,
        Method::POST,
        "/webhooks/poem-bot",
        None,
        Some(json!({ "trigger": "test" })),
    )
    .await;
    assert_eq!(
        wh_pre_st,
        StatusCode::FORBIDDEN,
        "webhook must be blocked prior to supervisor approval"
    );

    // 3. Publish HTML to render slug
    let (pub_st, pub_res) = send_json(
        &harness.router,
        Method::POST,
        &format!("/api/v1/workflows/{workflow_id}/render"),
        Some(&token),
        Some(json!({
            "content": "<html><body><h1>The Silent Night</h1><p>Moonlit shadows dance...</p></body></html>",
            "contentType": "text/html; charset=utf-8"
        })),
    )
    .await;
    assert_eq!(pub_st, StatusCode::OK, "render publish failed: {pub_res}");

    // 4. Access render slug BEFORE approval -> must be rejected 403 Forbidden
    let (render_pre_st, _render_pre_res) =
        send_json(&harness.router, Method::GET, "/render/poem-bot", None, None).await;
    assert_eq!(
        render_pre_st,
        StatusCode::FORBIDDEN,
        "render slug must be blocked prior to supervisor approval"
    );

    // 5. Supervisor approves workflow DAG (OWASP ASI-02/ASI-08 gate)
    let (approve_st, approve_res) = send_json(
        &harness.router,
        Method::POST,
        &format!("/api/v1/workflows/{workflow_id}/approve"),
        Some(&token),
        Some(json!({ "notes": "Verified poetry generation safety" })),
    )
    .await;
    assert_eq!(approve_st, StatusCode::OK, "approve failed: {approve_res}");
    assert_eq!(approve_res["approved"], true);

    // 6. Call dynamic webhook AFTER approval -> succeeds and runs workflow!
    let (wh_post_st, wh_post_res) = send_json(
        &harness.router,
        Method::POST,
        "/webhooks/poem-bot",
        None,
        Some(json!({ "trigger": "test-live" })),
    )
    .await;
    assert_eq!(wh_post_st, StatusCode::OK, "webhook failed: {wh_post_res}");
    assert_eq!(wh_post_res["status"], "completed");

    // 7. Access render slug AFTER approval -> succeeds with HTTP 200!
    let (render_post_st, render_post_res) =
        send_json(&harness.router, Method::GET, "/render/poem-bot", None, None).await;
    assert_eq!(
        render_post_st,
        StatusCode::OK,
        "render slug failed: {render_post_res}"
    );
}

#[tokio::test]
async fn http_request_node_workflow_executes_and_records_telemetry() {
    let harness = build_test_harness(ServerConfig::for_tests()).await;
    let (_user_id, token) =
        register_and_login(&harness.router, "monitor@domain.test", "P@ssword123456!").await;

    // Create a 5-minute health check workflow targeting an external/internal endpoint
    let (create_st, create_res) = send_json(
        &harness.router,
        Method::POST,
        "/api/v1/workflows",
        Some(&token),
        Some(json!({
            "name": "Domain Health Monitor",
            "nodes": [
                {
                    "nodeId": "probe_node",
                    "kind": "http_request",
                    "config": {
                        "url": "https://natuleadan.com",
                        "method": "GET"
                    }
                }
            ],
            "edges": []
        })),
    )
    .await;
    assert_eq!(create_st, StatusCode::OK);
    let workflow_id = create_res["workflowId"].as_str().unwrap();

    // Trigger workflow manually
    let (run_st, run_res) = send_json(
        &harness.router,
        Method::POST,
        &format!("/api/v1/workflows/{workflow_id}/trigger"),
        Some(&token),
        Some(json!({ "input": { "origin": "scheduled_monitor" } })),
    )
    .await;

    assert_eq!(run_st, StatusCode::OK, "trigger failed: {run_res}");
    assert_eq!(run_res["status"], "completed");

    // Verify checkpoint captured probe results
    let outputs = &run_res["checkpointState"]["outputs"]["probe_node"];
    assert!(outputs["latencyMs"].is_number());
}
