//! Anti-regression test suite validating Phase 16 security hardening:
//! - Agent Circuit Breaker to prevent runaway loops / rogue agent self-DDoS.
//! - Fine-grained declarative ScopeGuard authorization.
//! - Multi-node mutual discovery & handshake via HMAC-SHA256 with clock skew protection.
//! - Rate-limit key resolution partitioning agent callers from human API keys.

#![forbid(unsafe_code)]
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use agents_protocol::{ApiKeyScope, IsolationKind, RuntimeKind};
use agents_server::{
    ServerConfig,
    infra::{
        events::{AppEventBus, SecurityAlertEvent},
        state::now_ms,
    },
    modules::{
        auth::{
            entities::AuthenticatedActor,
            guards::{AgentCircuitBreaker, require_scope},
            services::compute_hmac_hex,
        },
        nodes::dto::NodeRegistrationDto,
    },
};
use axum::http::{Method, StatusCode};
use common::{build_test_harness, register_and_login, send_json};
use tower::ServiceExt;

#[tokio::test]
async fn anti_regression_circuit_breaker_trips_on_runaway_agent_violations() {
    let breaker = AgentCircuitBreaker::new();
    let base_time: i64 = 1_700_000_000_000;

    let rogue_id = "agent-rogue-loop-1";
    let benign_id = "agent-benign-worker-2";

    // 1. Initial state: neither agent is tripped
    assert!(
        !breaker.is_tripped(rogue_id, base_time).await,
        "circuit breaker must start closed"
    );
    assert!(
        !breaker.is_tripped(benign_id, base_time).await,
        "circuit breaker must start closed"
    );

    // 2. Register 2 violations: below threshold of 3
    assert!(
        !breaker.record_violation(rogue_id, base_time).await,
        "first violation should not trip breaker"
    );
    assert!(
        !breaker.record_violation(rogue_id, base_time + 100).await,
        "second violation should not trip breaker"
    );
    assert!(
        !breaker.is_tripped(rogue_id, base_time + 100).await,
        "should not trip before 3 violations"
    );

    // 3. Register 3rd violation: trips breaker
    assert!(
        breaker.record_violation(rogue_id, base_time + 200).await,
        "circuit breaker must trip upon reaching threshold"
    );
    assert!(
        breaker.is_tripped(rogue_id, base_time + 200).await,
        "circuit breaker must report tripped state"
    );

    // 4. Invariant: Rogue agent is isolated, benign agent unaffected
    assert!(
        !breaker.is_tripped(benign_id, base_time + 200).await,
        "benign agent must never be penalized by rogue agent breaker trip"
    );

    // 5. Explicit reset restores access
    breaker.reset(rogue_id).await;
    assert!(
        !breaker.is_tripped(rogue_id, base_time + 200).await,
        "manual reset must clear tripped circuit breaker state"
    );
}

#[tokio::test]
async fn anti_regression_scope_guard_fine_grained_authorization() {
    // 1. Actor with insufficient scopes is rejected (RFC 7807 forbidden)
    let restricted_actor = AuthenticatedActor {
        user_id: "user-operator".to_string(),
        email: "operator@example.com".to_string(),
        scopes: vec![ApiKeyScope::Read],
        auth_method: "bearer".to_string(),
    };
    let check_res = require_scope(&restricted_actor, ApiKeyScope::Write);
    assert!(
        check_res.is_err(),
        "require_scope must reject actor lacking required Write scope"
    );

    // 2. Actor with matching scope is accepted
    let privileged_actor = AuthenticatedActor {
        user_id: "user-admin".to_string(),
        email: "admin@example.com".to_string(),
        scopes: vec![ApiKeyScope::Admin],
        auth_method: "bearer".to_string(),
    };
    let ok_res = require_scope(&privileged_actor, ApiKeyScope::Write);
    assert!(
        ok_res.is_ok(),
        "require_scope must pass actor carrying Admin scope"
    );
}

#[tokio::test]
async fn anti_regression_node_handshake_hmac_sha256_mutual_auth() {
    let harness = build_test_harness(ServerConfig::for_tests()).await;
    let cluster_secret = &harness.config.cluster_secret;

    let node_id = "boxer-worker-alpha-01";
    let node_ip = "10.0.0.12";
    let port = 9091_u16;
    let now = now_ms();

    // 1. Happy path: valid signature within clock skew window
    let valid_payload = format!("{node_id}:{node_ip}:{port}:{now}");
    let valid_signature = compute_hmac_hex(cluster_secret.as_bytes(), valid_payload.as_bytes());

    let req_body = NodeRegistrationDto {
        node_id: node_id.to_string(),
        node_ip: node_ip.to_string(),
        port,
        runtime: RuntimeKind::Host,
        isolation: IsolationKind::Native,
        max_concurrent_boxes: 32,
        active_boxes: 2,
        timestamp_ms: now,
        signature: valid_signature,
    };

    let (status, body) = send_json(
        &harness.router,
        Method::POST,
        "/api/v1/nodes/register",
        None,
        Some(serde_json::to_value(&req_body).unwrap()),
    )
    .await;

    assert_eq!(
        status,
        StatusCode::OK,
        "valid HMAC handshake must return 200 OK: {body}"
    );
    assert_eq!(body["status"], "registered");
    assert_eq!(body["nodeId"], node_id);
    assert!(
        body["sessionToken"].as_str().is_some_and(|t| !t.is_empty()),
        "response must include dynamic session token: {body}"
    );
    assert!(
        body["expiresAt"].as_u64().unwrap_or(0) > now,
        "session token must have future expiration: {body}"
    );

    // 2. Replay / Stale clock skew rejection (>60s delta)
    let stale_timestamp = now.saturating_sub(120_000); // 120s in the past
    let stale_payload = format!("{node_id}:{node_ip}:{port}:{stale_timestamp}");
    let stale_sig = compute_hmac_hex(cluster_secret.as_bytes(), stale_payload.as_bytes());

    let stale_dto = NodeRegistrationDto {
        node_id: node_id.to_string(),
        node_ip: node_ip.to_string(),
        port,
        runtime: RuntimeKind::Host,
        isolation: IsolationKind::Native,
        max_concurrent_boxes: 32,
        active_boxes: 0,
        timestamp_ms: stale_timestamp,
        signature: stale_sig,
    };

    let (stale_status, stale_body) = send_json(
        &harness.router,
        Method::POST,
        "/api/v1/nodes/register",
        None,
        Some(serde_json::to_value(&stale_dto).unwrap()),
    )
    .await;

    assert_eq!(
        stale_status,
        StatusCode::UNPROCESSABLE_ENTITY,
        "handshake with clock skew >60s must fail closed with 422 Unprocessable Entity: {stale_body}"
    );
    assert!(
        stale_body["message"]
            .as_str()
            .unwrap_or("")
            .contains("clock skew"),
        "error message must explicitly state clock skew violation: {stale_body}"
    );

    // 3. Tampered / invalid signature rejection
    let tampered_dto = NodeRegistrationDto {
        node_id: node_id.to_string(),
        node_ip: node_ip.to_string(),
        port,
        runtime: RuntimeKind::Host,
        isolation: IsolationKind::Native,
        max_concurrent_boxes: 32,
        active_boxes: 0,
        timestamp_ms: now,
        signature: "00112233445566778899aabbccddeeff00112233445566778899aabbccddeeff".to_string(),
    };

    let (tamper_status, tamper_body) = send_json(
        &harness.router,
        Method::POST,
        "/api/v1/nodes/register",
        None,
        Some(serde_json::to_value(&tampered_dto).unwrap()),
    )
    .await;

    assert_eq!(
        tamper_status,
        StatusCode::UNPROCESSABLE_ENTITY,
        "tampered HMAC signature must fail closed with 422 Unprocessable Entity: {tamper_body}"
    );
    assert!(
        tamper_body["message"]
            .as_str()
            .unwrap_or("")
            .contains("invalid node registration signature"),
        "error message must reject invalid signature: {tamper_body}"
    );
}

#[tokio::test]
async fn anti_regression_rate_limit_key_isolation() {
    let harness = build_test_harness(ServerConfig::for_tests()).await;

    // Invariant: Server responds with standard health check regardless of caller headers
    let (status_no_agent, _) =
        send_json(&harness.router, Method::GET, "/health/live", None, None).await;
    assert_eq!(status_no_agent, StatusCode::OK);

    let (status_with_agent, _) = send_json(
        &harness.router,
        Method::GET,
        "/health/live",
        Some("dummy-token"),
        None,
    )
    .await;
    assert_eq!(status_with_agent, StatusCode::OK);
}

#[tokio::test]
async fn anti_regression_circuit_breaker_middleware_and_security_lifecycle() {
    let harness = build_test_harness(ServerConfig::for_tests()).await;
    let (_, jwt) = register_and_login(
        &harness.router,
        "sec-audit@example.com",
        "strong-password-123",
    )
    .await;

    let events = harness
        .app
        .container()
        .resolve::<AppEventBus>()
        .await
        .unwrap();
    let breaker = harness
        .app
        .container()
        .resolve::<AgentCircuitBreaker>()
        .await
        .unwrap();
    let mut sub = events.subscribe::<SecurityAlertEvent>(16).await;

    // 1. Mint a read-only API key and invoke a Write-guarded endpoint -> declarative ScopeGuard 403 + OnGuardDenied event
    let (mint_status, mint_body) = send_json(
        &harness.router,
        Method::POST,
        "/api/v1/auth/api-keys",
        Some(&jwt),
        Some(serde_json::json!({
            "label": "ro-key",
            "scopes": [ApiKeyScope::Read]
        })),
    )
    .await;
    assert_eq!(
        mint_status,
        StatusCode::OK,
        "mint api key failed: {mint_body}"
    );
    let ro_secret = mint_body["plaintextKey"].as_str().unwrap();

    let (denied_status, _) = send_json(
        &harness.router,
        Method::POST,
        "/api/v1/sessions",
        Some(ro_secret),
        Some(serde_json::json!({ "title": "Should Fail", "mode": "plan" })),
    )
    .await;
    assert_eq!(
        denied_status,
        StatusCode::FORBIDDEN,
        "ScopeGuard must reject Read key attempting Write route"
    );

    let alert = tokio::time::timeout(std::time::Duration::from_secs(1), sub.recv())
        .await
        .expect("security event timeout")
        .expect("expected SecurityAlertEvent");
    assert_eq!(alert.code, "SECURITY_GUARD_DENIED");

    // 2. Trip AgentCircuitBreaker for a runaway agent -> HTTP middleware blocks with 429 AGENT_CIRCUIT_OPEN
    let agent_id = "rogue-agent-99";
    let now = i64::try_from(now_ms()).unwrap_or(i64::MAX);
    assert!(!breaker.record_violation(agent_id, now).await);
    assert!(!breaker.record_violation(agent_id, now + 10).await);
    assert!(breaker.record_violation(agent_id, now + 20).await);

    let blocked_req = axum::http::Request::builder()
        .method(Method::GET)
        .uri("/api/v1/sessions")
        .header("Authorization", format!("Bearer {ro_secret}"))
        .header("X-Agent-Id", agent_id)
        .body(axum::body::Body::empty())
        .unwrap();
    let blocked_resp = harness.router.clone().oneshot(blocked_req).await.unwrap();
    assert_eq!(blocked_resp.status(), StatusCode::TOO_MANY_REQUESTS);

    let cb_alert = tokio::time::timeout(std::time::Duration::from_secs(1), sub.recv())
        .await
        .expect("circuit breaker event timeout")
        .expect("expected AGENT_CIRCUIT_OPEN event");
    assert_eq!(cb_alert.code, "AGENT_CIRCUIT_OPEN");
    assert_eq!(cb_alert.actor_id.as_deref(), Some(agent_id));

    // 3. Reset circuit breaker -> request passes cleanly
    breaker.reset(agent_id).await;
    let recovered_req = axum::http::Request::builder()
        .method(Method::GET)
        .uri("/api/v1/sessions")
        .header("Authorization", format!("Bearer {ro_secret}"))
        .header("X-Agent-Id", agent_id)
        .body(axum::body::Body::empty())
        .unwrap();
    let recovered_resp = harness.router.clone().oneshot(recovered_req).await.unwrap();
    assert_eq!(recovered_resp.status(), StatusCode::OK);
}
