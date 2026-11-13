#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use agents_protocol::{API_GENERATION, Capability, CodecId, HandshakeRequest, StreamPayload};
use agents_server::{ServerConfig, modules::gateway::services::GatewayService};
use axum::{
    body::Body,
    http::{Method, Request, StatusCode},
};
use futures_util::{SinkExt, StreamExt};
use http_body_util::BodyExt;
use serde_json::{Value, json};
use tokio::net::TcpListener;
use tokio_tungstenite::{connect_async, tungstenite::Message};
use tower::ServiceExt;

use common::{build_test_harness, build_test_harness_with_config, register_and_login, send_json};

#[tokio::test]
async fn health_openapi_and_sse_replay_enforce_auth_and_order() {
    let harness = build_test_harness(ServerConfig::for_tests()).await;
    let (_, alice_jwt) =
        register_and_login(&harness.router, "alice@example.com", "password-alice-123").await;
    let (_, bob_jwt) =
        register_and_login(&harness.router, "bob@example.com", "password-bob-1234").await;

    // Health and OpenAPI endpoints work out of the box (PostgreSQL unset -> /health is 207 degraded, /health/ready is 200).
    for path in ["/health/live", "/health/ready", "/openapi.json"] {
        let req = Request::builder().uri(path).body(Body::empty()).unwrap();
        let resp = harness.router.clone().oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK, "endpoint {path} failed");
    }
    let health_req = Request::builder()
        .uri("/health")
        .body(Body::empty())
        .unwrap();
    let health_resp = harness.router.clone().oneshot(health_req).await.unwrap();
    assert!(
        matches!(
            health_resp.status(),
            StatusCode::OK | StatusCode::MULTI_STATUS
        ),
        "endpoint /health returned unexpected {}",
        health_resp.status()
    );

    let (_, session) = send_json(
        &harness.router,
        Method::POST,
        "/api/v1/sessions",
        Some(&alice_jwt),
        Some(json!({ "title": "Streaming session" })),
    )
    .await;
    let sid = session["id"].as_str().unwrap().to_owned();

    // Enqueue 2 tasks so stream_events contains seq=1 and seq=2.
    for prompt in ["First", "Second"] {
        let (st, _) = send_json(
            &harness.router,
            Method::POST,
            &format!("/api/v1/sessions/{sid}/tasks"),
            Some(&alice_jwt),
            Some(json!({ "prompt": prompt })),
        )
        .await;
        assert_eq!(st, StatusCode::OK);
    }

    // Replay endpoint filters after_seq=1 and rejects Bob with 403.
    let (bob_replay, _) = send_json(
        &harness.router,
        Method::GET,
        &format!("/api/v1/sessions/{sid}/events"),
        Some(&bob_jwt),
        None,
    )
    .await;
    assert_eq!(bob_replay, StatusCode::FORBIDDEN);

    let (replay_st, replay_body) = send_json(
        &harness.router,
        Method::GET,
        &format!("/api/v1/sessions/{sid}/events?afterSeq=1"),
        Some(&alice_jwt),
        None,
    )
    .await;
    assert_eq!(replay_st, StatusCode::OK);
    let items = replay_body.as_array().unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["seq"], 2);

    // Spawn ephemeral listener to verify SSE stream with Last-Event-ID.
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let router_clone = harness.router.clone();
    let srv = tokio::spawn(async move {
        let _ = axum::serve(listener, router_clone).await;
    });

    // Unauthenticated SSE returns 401.
    let unauth_resp = harness
        .router
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!("/api/v1/sessions/{sid}/events/stream"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(unauth_resp.status(), StatusCode::UNAUTHORIZED);

    // Authenticated SSE emits event seq=2 when Last-Event-ID: 1 is passed.
    let sse_req = Request::builder()
        .uri(format!("/api/v1/sessions/{sid}/events/stream"))
        .header("authorization", format!("Bearer {alice_jwt}"))
        .header("last-event-id", "1")
        .body(Body::empty())
        .unwrap();
    let sse_resp = harness.router.clone().oneshot(sse_req).await.unwrap();
    assert_eq!(sse_resp.status(), StatusCode::OK);
    let mut sse_body = sse_resp.into_body();
    let first_chunk = tokio::time::timeout(std::time::Duration::from_secs(2), sse_body.frame())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    let text = String::from_utf8_lossy(first_chunk.data_ref().unwrap()).to_string();
    assert!(text.contains("id: 2"), "expected SSE id: 2, got {text}");
    let _ = addr;
    srv.abort();
}

#[tokio::test]
async fn websocket_gateway_handshake_steer_and_pause_resume_reconciliation() {
    let harness = build_test_harness(ServerConfig::for_tests()).await;
    let (alice_id, alice_jwt) =
        register_and_login(&harness.router, "alice@example.com", "password-alice-123").await;
    let (_, bob_jwt) =
        register_and_login(&harness.router, "bob@example.com", "password-bob-1234").await;

    let (_, session) = send_json(
        &harness.router,
        Method::POST,
        "/api/v1/sessions",
        Some(&alice_jwt),
        Some(json!({ "title": "WS room" })),
    )
    .await;
    let sid = session["id"].as_str().unwrap().to_owned();

    let (_, task) = send_json(
        &harness.router,
        Method::POST,
        &format!("/api/v1/sessions/{sid}/tasks"),
        Some(&alice_jwt),
        Some(json!({ "prompt": "Start WS test task" })),
    )
    .await;
    let tid = task["id"].as_str().unwrap().to_owned();

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let router_clone = harness.router.clone();
    let srv = tokio::spawn(async move {
        let _ = axum::serve(listener, router_clone).await;
    });

    // Cross-user WS upgrade is rejected with 403 Forbidden.
    let bob_req = tokio_tungstenite::tungstenite::ClientRequestBuilder::new(
        format!("ws://{addr}/api/v1/sessions/{sid}/ws")
            .parse()
            .unwrap(),
    )
    .with_header("Authorization", format!("Bearer {bob_jwt}"));
    let bob_err = connect_async(bob_req).await.unwrap_err();
    match &bob_err {
        tokio_tungstenite::tungstenite::Error::Http(resp) => {
            let body_txt = resp
                .body()
                .as_deref()
                .map(|b| String::from_utf8_lossy(b).to_string())
                .unwrap_or_default();
            assert_eq!(
                resp.status().as_u16(),
                403,
                "unexpected WS status with body: {body_txt}"
            );
        }
        other => panic!("expected HTTP 403 rejection on cross-user WS, got: {other:?}"),
    }

    // Authenticated WS connection receives backlog event seq=1.
    let alice_req = tokio_tungstenite::tungstenite::ClientRequestBuilder::new(
        format!("ws://{addr}/api/v1/sessions/{sid}/ws")
            .parse()
            .unwrap(),
    )
    .with_header("Authorization", format!("Bearer {alice_jwt}"));
    let (mut ws, _) = connect_async(alice_req).await.unwrap();

    let backlog_msg = ws.next().await.unwrap().unwrap();
    let backlog_json: Value = serde_json::from_str(backlog_msg.to_text().unwrap()).unwrap();
    assert_eq!(backlog_json["type"], "event");
    assert_eq!(backlog_json["seq"], 1);

    // Send Handshake request and verify capabilities & generation.
    let handshake = HandshakeRequest {
        client_name: "nsh-test".to_owned(),
        client_version: "0.1.0".to_owned(),
        api_generation: API_GENERATION,
        requested_codecs: vec![CodecId::EventsV1],
        requested_capabilities: vec![Capability::LiveSteering, Capability::Swarm],
    };
    ws.send(Message::Text(
        json!({ "type": "handshake", "clientName": handshake.client_name, "clientVersion": handshake.client_version, "apiGeneration": handshake.api_generation, "requestedCodecs": handshake.requested_codecs, "requestedCapabilities": handshake.requested_capabilities })
            .to_string()
            .into(),
    ))
    .await
    .unwrap();
    let hs_reply: Value =
        serde_json::from_str(ws.next().await.unwrap().unwrap().to_text().unwrap()).unwrap();
    assert_eq!(hs_reply["type"], "handshake");
    assert_eq!(hs_reply["apiGeneration"], API_GENERATION);

    // Inject live Steer frame over WS and confirm bus records it.
    ws.send(Message::Text(
        json!({ "type": "steer", "taskId": tid, "content": "Adjust step 2" })
            .to_string()
            .into(),
    ))
    .await
    .unwrap();
    let steer_ack: Value =
        serde_json::from_str(ws.next().await.unwrap().unwrap().to_text().unwrap()).unwrap();
    assert_eq!(steer_ack["type"], "ack");
    assert_eq!(steer_ack["action"], "steer");
    assert_eq!(harness.bus.recorded_steering().await.len(), 1);

    // Pause stream, draining any queued live steeringAck envelope first.
    ws.send(Message::Text(json!({ "type": "pause" }).to_string().into()))
        .await
        .unwrap();
    let pause_ack = loop {
        let frame: Value =
            serde_json::from_str(ws.next().await.unwrap().unwrap().to_text().unwrap()).unwrap();
        if frame["type"] == "event" {
            assert_eq!(frame["payload"]["kind"], "steeringAck");
            continue;
        }
        break frame;
    };
    assert_eq!(pause_ack["type"], "ack");
    assert_eq!(pause_ack["action"], "pause");

    let gw = harness
        .app
        .container()
        .resolve::<GatewayService>()
        .await
        .unwrap();
    let actor = agents_server::modules::auth::entities::AuthenticatedActor {
        user_id: alice_id,
        email: "alice@example.com".to_owned(),
        scopes: vec![agents_protocol::ApiKeyScope::Write],
        auth_method: "jwt".to_owned(),
    };
    gw.emit_event(
        &actor,
        &sid,
        Some(&tid),
        StreamPayload::TaskChunk {
            chunk: "emitted-while-paused".to_owned(),
            is_stderr: false,
        },
    )
    .await
    .unwrap();

    ws.send(Message::Text(
        json!({ "type": "resume" }).to_string().into(),
    ))
    .await
    .unwrap();
    let resume_ack: Value =
        serde_json::from_str(ws.next().await.unwrap().unwrap().to_text().unwrap()).unwrap();
    assert_eq!(resume_ack["action"], "resume");

    let missed: Value =
        serde_json::from_str(ws.next().await.unwrap().unwrap().to_text().unwrap()).unwrap();
    assert_eq!(missed["type"], "event");
    assert_eq!(missed["payload"]["kind"], "taskChunk");
    assert_eq!(missed["payload"]["chunk"], "emitted-while-paused");

    srv.abort();
}

#[tokio::test]
async fn mcp_host_enforces_bearer_auth_allowlist_scope_bola_and_rate_limits() {
    let mut cfg = ServerConfig::for_tests();
    cfg.rate_limit_max_requests = 5;
    cfg.rate_limit_window_secs = 60;
    let harness = build_test_harness_with_config(cfg).await;

    let (_, alice_jwt) =
        register_and_login(&harness.router, "alice@example.com", "password-alice-123").await;
    let (_, bob_jwt) =
        register_and_login(&harness.router, "bob@example.com", "password-bob-1234").await;

    let (_, alice_session) = send_json(
        &harness.router,
        Method::POST,
        "/api/v1/sessions",
        Some(&alice_jwt),
        Some(json!({ "title": "Alice MCP session" })),
    )
    .await;
    let alice_sid = alice_session["id"].as_str().unwrap().to_owned();

    // Unauthenticated POST /mcp returns 401.
    let (unauth_st, _) = send_json(
        &harness.router,
        Method::POST,
        "/mcp",
        None,
        Some(json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/list" })),
    )
    .await;
    assert_eq!(unauth_st, StatusCode::UNAUTHORIZED);

    // Inspect policy metadata endpoint.
    let (pol_st, pol_body) = send_json(
        &harness.router,
        Method::GET,
        "/api/v1/mcp/policy",
        Some(&alice_jwt),
        None,
    )
    .await;
    assert_eq!(pol_st, StatusCode::OK);
    assert!(pol_body["allowedTools"].as_array().unwrap().len() >= 5);

    // tools/list over MCP returns registered tools.
    let (list_st, list_body) = send_json(
        &harness.router,
        Method::POST,
        "/mcp",
        Some(&alice_jwt),
        Some(json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/list" })),
    )
    .await;
    assert_eq!(list_st, StatusCode::OK);
    assert!(list_body["result"]["tools"].as_array().unwrap().len() >= 5);

    // Disallowed tool returns 403 Forbidden.
    let (deny_st, _) = send_json(
        &harness.router,
        Method::POST,
        "/mcp",
        Some(&alice_jwt),
        Some(json!({
            "jsonrpc": "2.0",
            "id": 2,
            "method": "tools/call",
            "params": { "name": "shell_exec", "arguments": {} }
        })),
    )
    .await;
    assert_eq!(deny_st, StatusCode::FORBIDDEN);

    // Allowed `task_submit` creates a task for Alice.
    let (sub_st, sub_body) = send_json(
        &harness.router,
        Method::POST,
        "/mcp",
        Some(&alice_jwt),
        Some(json!({
            "jsonrpc": "2.0",
            "id": 3,
            "method": "tools/call",
            "params": {
                "name": "task_submit",
                "arguments": { "sessionId": alice_sid, "prompt": "Via MCP" }
            }
        })),
    )
    .await;
    assert_eq!(sub_st, StatusCode::OK);
    assert!(
        sub_body["error"].is_null(),
        "unexpected MCP error: {sub_body}"
    );

    // Bob's `session_get` on Alice's session fails BOLA inside MCP handler.
    let (bola_st, bola_body) = send_json(
        &harness.router,
        Method::POST,
        "/mcp",
        Some(&bob_jwt),
        Some(json!({
            "jsonrpc": "2.0",
            "id": 4,
            "method": "tools/call",
            "params": {
                "name": "session_get",
                "arguments": { "sessionId": alice_sid }
            }
        })),
    )
    .await;
    assert_eq!(bola_st, StatusCode::OK);
    assert!(!bola_body["error"].is_null());

    // Exhaust Alice's MCP rate limit (max=5) -> 429 Too Many Requests.
    let mut hit_rate_limit = false;
    for i in 10..20 {
        let (st, _) = send_json(
            &harness.router,
            Method::POST,
            "/mcp",
            Some(&alice_jwt),
            Some(json!({ "jsonrpc": "2.0", "id": i, "method": "tools/list" })),
        )
        .await;
        if st == StatusCode::TOO_MANY_REQUESTS {
            hit_rate_limit = true;
            break;
        }
    }
    assert!(hit_rate_limit, "expected MCP rate limiter to return 429");
}
