#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use agents_protocol::{
    IsolationKind, NodeStatus, PermissionMask, RuntimeKind, SandboxProfile, SetupMode,
    TraefikPreviewRoute,
};
use agents_server::{
    ServerConfig,
    infra::config::ConfigError,
    modules::auth::services::{constant_time_eq_hex, sha256_hex},
};
use axum::{
    body::Body,
    http::{Method, Request, StatusCode},
};
use tower::ServiceExt;

use common::{build_test_harness, register_and_login, send_json};
use serde_json::json;

#[tokio::test]
async fn owasp_top10_2025_bola_crypto_headers_ssrf_and_node_discovery() {
    let harness = build_test_harness(ServerConfig::for_tests()).await;
    let (_, alice_jwt) = register_and_login(
        &harness.router,
        "alice-owasp@example.com",
        "strong-alice-pass-99",
    )
    .await;
    let (_, bob_jwt) = register_and_login(
        &harness.router,
        "bob-owasp@example.com",
        "strong-bob-pass-99",
    )
    .await;

    // A05 / Security Misconfiguration: Security headers must be present on HTTP responses.
    let req = Request::builder()
        .uri("/health/live")
        .body(Body::empty())
        .unwrap();
    let live_resp = harness.router.clone().oneshot(req).await.unwrap();
    assert_eq!(live_resp.status(), StatusCode::OK);
    assert!(live_resp.headers().contains_key("x-content-type-options"));
    assert!(live_resp.headers().contains_key("x-frame-options"));

    // A07 / Auth: Forged JWT is rejected cleanly with 401.
    let (forged_st, _) = send_json(
        &harness.router,
        Method::GET,
        "/api/v1/nodes",
        Some("eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiJmYWtlIn0.bad-signature"),
        None,
    )
    .await;
    assert_eq!(forged_st, StatusCode::UNAUTHORIZED);

    // A01 / API1 BOLA: Alice creates session; Bob is blocked with 403 on read & mutation.
    let (sess_st, sess_body) = send_json(
        &harness.router,
        Method::POST,
        "/api/v1/sessions",
        Some(&alice_jwt),
        Some(json!({ "title": "OWASP tenant session", "mode": "act" })),
    )
    .await;
    assert_eq!(sess_st, StatusCode::OK);
    let sid = sess_body["id"].as_str().unwrap();

    let (bob_sess_st, _) = send_json(
        &harness.router,
        Method::GET,
        &format!("/api/v1/sessions/{sid}"),
        Some(&bob_jwt),
        None,
    )
    .await;
    assert_eq!(bob_sess_st, StatusCode::FORBIDDEN);

    let (bob_task_st, _) = send_json(
        &harness.router,
        Method::POST,
        &format!("/api/v1/sessions/{sid}/tasks"),
        Some(&bob_jwt),
        Some(json!({ "prompt": "unauthorized cross-tenant execution" })),
    )
    .await;
    assert_eq!(bob_task_st, StatusCode::FORBIDDEN);

    // A10 / SSRF & VLAN Isolation: Non-VLAN public IP rejected fail-closed on Boxer discovery & preview sync.
    let (bad_node_st, _) = send_json(
        &harness.router,
        Method::POST,
        "/api/v1/nodes/heartbeat",
        Some(&alice_jwt),
        Some(json!({
            "nodeId": "boxer-public-bad",
            "vlanIp": "8.8.8.8",
            "runtime": RuntimeKind::Docker,
            "isolation": IsolationKind::Openshell,
            "maxConcurrentBoxes": 4,
            "activeBoxes": 0,
            "status": NodeStatus::Online,
            "ttlSeconds": 60
        })),
    )
    .await;
    assert_eq!(bad_node_st, StatusCode::UNPROCESSABLE_ENTITY);

    let (bad_preview_st, _) = send_json(
        &harness.router,
        Method::POST,
        "/api/v1/nodes/previews",
        Some(&alice_jwt),
        Some(json!({
            "route": TraefikPreviewRoute {
                route_id: "route-ssrf".to_owned(),
                task_id: "task-1".to_owned(),
                slug: "ssrf-target".to_owned(),
                internal_host: "1.1.1.1".to_owned(),
                internal_port: 80,
                public_url: "http://127.0.0.1/preview/ssrf-target".to_owned(),
                ttl_seconds: 120,
                requires_token: false,
                created_at_ms: 9_999_999_999_999,
            },
            "tokenSha256": null,
            "revoked": false
        })),
    )
    .await;
    assert_eq!(bad_preview_st, StatusCode::UNPROCESSABLE_ENTITY);

    // Multi-Boxer infinite VLAN auto-registration over 10.0.0.0/8.
    for (id, ip) in [("boxer-mia-01", "10.0.1.10"), ("boxer-mia-02", "10.0.1.11")] {
        let (st, node) = send_json(
            &harness.router,
            Method::POST,
            "/api/v1/nodes/heartbeat",
            Some(&alice_jwt),
            Some(json!({
                "nodeId": id,
                "vlanIp": ip,
                "runtime": RuntimeKind::Docker,
                "isolation": IsolationKind::Openshell,
                "maxConcurrentBoxes": 8,
                "activeBoxes": 1,
                "status": NodeStatus::Online,
                "ttlSeconds": 120
            })),
        )
        .await;
        assert_eq!(st, StatusCode::OK);
        assert_eq!(node["nodeId"], id);
    }

    let (list_st, list_body) = send_json(
        &harness.router,
        Method::GET,
        "/api/v1/nodes",
        Some(&alice_jwt),
        None,
    )
    .await;
    assert_eq!(list_st, StatusCode::OK);
    assert_eq!(list_body["total"], 2);

    // Distributed Mode B preview routing via /internal/traefik/dynamic.
    let (pub_st, pub_body) = send_json(
        &harness.router,
        Method::POST,
        "/api/v1/nodes/previews",
        Some(&alice_jwt),
        Some(json!({
            "route": TraefikPreviewRoute {
                route_id: "route-mia-01".to_owned(),
                task_id: "task-burger".to_owned(),
                slug: "burger-demo".to_owned(),
                internal_host: "10.0.1.11".to_owned(),
                internal_port: 3000,
                public_url: "http://127.0.0.1/preview/burger-demo".to_owned(),
                ttl_seconds: 300,
                requires_token: true,
                created_at_ms: 9_999_999_999_999,
            },
            "tokenSha256": "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
            "revoked": false
        })),
    )
    .await;
    assert_eq!(pub_st, StatusCode::OK);
    assert_eq!(pub_body["active"], true);

    let (dyn_st, dyn_body) = send_json(
        &harness.router,
        Method::GET,
        "/internal/traefik/dynamic",
        None,
        None,
    )
    .await;
    assert_eq!(dyn_st, StatusCode::OK);
    assert_eq!(
        dyn_body["http"]["services"]["boxer-svc-route-mia-01"]["loadBalancer"]["servers"][0]["url"],
        "http://10.0.1.11:3000"
    );

    // Revoking preview tears down dynamic Traefik route cleanly.
    let (rev_st, rev_body) = send_json(
        &harness.router,
        Method::POST,
        "/api/v1/nodes/previews",
        Some(&alice_jwt),
        Some(json!({
            "route": TraefikPreviewRoute {
                route_id: "route-mia-01".to_owned(),
                task_id: "task-burger".to_owned(),
                slug: "burger-demo".to_owned(),
                internal_host: "10.0.1.11".to_owned(),
                internal_port: 3000,
                public_url: "http://127.0.0.1/preview/burger-demo".to_owned(),
                ttl_seconds: 300,
                requires_token: true,
                created_at_ms: 9_999_999_999_999,
            },
            "tokenSha256": null,
            "revoked": true
        })),
    )
    .await;
    assert_eq!(rev_st, StatusCode::OK);
    assert_eq!(rev_body["active"], false);

    // A09 / Admin-only RBAC rejects non-admin user on audit verification, while hash chain remains intact.
    let (audit_denied_st, _) = send_json(
        &harness.router,
        Method::GET,
        "/api/v1/audit/verify",
        Some(&alice_jwt),
        None,
    )
    .await;
    assert_eq!(audit_denied_st, StatusCode::FORBIDDEN);

    let audit_svc = harness
        .app
        .container()
        .resolve::<agents_server::modules::audit::services::AuditService>()
        .await
        .unwrap();
    let report = audit_svc.verify_chain().await.unwrap();
    assert!(report.valid && report.checked_entries >= 4);
}

#[test]
fn owasp_asi_2026_and_fail_closed_crypto_invariants() {
    // Production config fails closed without >=32 byte secret and Postgres URL.
    let weak = ServerConfig::from_lookup(|k| match k {
        "AGENTS_ENV" => Some("production".to_owned()),
        "AGENTS_JWT_SECRET" => Some("short".to_owned()),
        _ => None,
    });
    assert!(matches!(weak, Err(ConfigError::WeakJwtSecret)));

    // ASI-01 / ASI-02: Native host runtime without explicit allow_host_native fails closed.
    let unsafe_profile = SandboxProfile {
        mode: SetupMode::Local,
        runtime: RuntimeKind::Host,
        isolation: IsolationKind::Native,
        allow_host_native: false,
        gateway_url: None,
    };
    assert!(unsafe_profile.validate().is_err());

    // ASI-07: Parent permission mask strictly blocks child privilege escalation.
    let parent = PermissionMask {
        can_read_fs: true,
        can_write_fs: false,
        can_exec_shell: false,
        can_network: false,
        allowed_tools: vec!["read".to_owned()],
    };
    let mut escalate = parent.clone();
    escalate.can_exec_shell = true;
    assert!(parent.narrow(&escalate).is_err());

    // A04: Constant-time digest comparison matches exact SHA-256 and rejects tampered values.
    let expected = sha256_hex("opaque_key_value");
    let candidate = sha256_hex("opaque_key_value");
    let tampered = sha256_hex("opaque_key_tampered");
    assert!(constant_time_eq_hex(&expected, &candidate));
    assert!(!constant_time_eq_hex(&expected, &tampered));
}
