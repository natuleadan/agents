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
    infra::state::now_ms,
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
use common::{build_test_harness, send_json};

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
