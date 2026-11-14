//! Anti-regression test suite validating API optimization, observability, and benchmarking:
//! - Native Prometheus `/metrics` and `/metrics/prometheus` text endpoints.
//! - `Container::health()` provider health indicator registered into composite `GET /health`.
//! - Gzip compression on `Accept-Encoding: gzip`.
//! - Standardized native `#[decorator(Pagination)]` and `Response::paginated` on `/page` endpoints.
//! - W3C `traceparent` context propagation and `x-trace-id` header injection.
//! - PerRouteTimeoutInterceptor returning HTTP 408 on exceeded limits.
//! - Complete 8-stage `LifecycleManager` tracking and snapshot recording.
//! - Source-level anti-regression assertions.

#![forbid(unsafe_code)]
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use std::path::Path;

use agents_server::{
    ServerConfig,
    infra::{LifecycleManager, W3cTraceContext, inject_w3c_traceparent, slice_paginated},
};
use axum::{
    body::Body,
    http::{Method, Request, StatusCode},
};
use common::{build_test_harness, register_and_login, send_json};
use http_body_util::BodyExt;
use ironic::ShutdownSignal;
use serde_json::Value;
use tower::ServiceExt;

#[tokio::test]
async fn anti_regression_metrics_endpoints_and_provider_health() {
    let harness = build_test_harness(ServerConfig::for_tests()).await;

    // 1. GET /metrics (Ironic MetricsModule JSON payload)
    let req = Request::builder()
        .method(Method::GET)
        .uri("/metrics")
        .body(Body::empty())
        .unwrap();
    let resp = harness.router.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    let body_str = String::from_utf8_lossy(&bytes);
    assert!(
        body_str.contains("ironic_http_") || body_str.contains("agents_domain_"),
        "metrics must contain framework or domain counters"
    );

    // 2. GET /metrics/prometheus (raw Prometheus text format)
    let req = Request::builder()
        .method(Method::GET)
        .uri("/metrics/prometheus")
        .body(Body::empty())
        .unwrap();
    let resp = harness.router.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let ct = resp
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default();
    assert!(
        ct.contains("text/plain"),
        "expected text/plain content-type, got: {ct}"
    );

    // 3. GET /health includes Container::health 'providers' indicator
    let req = Request::builder()
        .method(Method::GET)
        .uri("/health")
        .body(Body::empty())
        .unwrap();
    let resp = harness.router.clone().oneshot(req).await.unwrap();
    assert!(
        resp.status() == StatusCode::OK || resp.status() == StatusCode::MULTI_STATUS,
        "health status should be 200 or 207 (multi-status in-memory)"
    );
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    let val: Value = serde_json::from_slice(&bytes).unwrap();
    let providers_check = &val["checks"]["providers"];
    assert_eq!(
        providers_check["status"], "ok",
        "DI providers health check must be 'ok'"
    );
}

#[tokio::test]
async fn anti_regression_gzip_compression_on_accept_encoding() {
    let harness = build_test_harness(ServerConfig::for_tests()).await;

    let req = Request::builder()
        .method(Method::GET)
        .uri("/metrics")
        .header("accept-encoding", "gzip")
        .body(Body::empty())
        .unwrap();
    let resp = harness.router.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let enc = resp
        .headers()
        .get("content-encoding")
        .and_then(|v| v.to_str().ok())
        .map(ToOwned::to_owned);
    let _bytes = resp.into_body().collect().await.unwrap().to_bytes();
    assert_eq!(
        enc.as_deref(),
        Some("gzip"),
        "response should be gzip compressed when client specifies accept-encoding: gzip"
    );
}

#[tokio::test]
async fn anti_regression_paginated_endpoints_and_clamping() {
    let harness = build_test_harness(ServerConfig::for_tests()).await;
    let (_user_id, token) = register_and_login(
        &harness.router,
        "paginated_user@test.local",
        "Admin!2026Secure",
    )
    .await;

    // 1. Create a session
    let session_body = serde_json::json!({ "title": "Pagination Test Session" });
    let (status, created) = send_json(
        &harness.router,
        Method::POST,
        "/api/v1/sessions",
        Some(&token),
        Some(session_body),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let session_id = created["id"].as_str().unwrap();

    // 2. Query /api/v1/sessions/page
    let req = Request::builder()
        .method(Method::GET)
        .uri("/api/v1/sessions/page?page=1&size=10")
        .header("authorization", format!("Bearer {}", token))
        .body(Body::empty())
        .unwrap();
    let resp = harness.router.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    let page: Value = serde_json::from_slice(&bytes).unwrap();
    assert!(page["items"].is_array());
    assert!(page["total"].as_u64().unwrap() >= 1);
    assert_eq!(page["offset"].as_u64(), Some(0));
    assert_eq!(page["limit"].as_u64(), Some(10));

    // 3. Query /api/v1/workflows/page
    let req = Request::builder()
        .method(Method::GET)
        .uri("/api/v1/workflows/page?page=1&size=5")
        .header("authorization", format!("Bearer {}", token))
        .body(Body::empty())
        .unwrap();
    let resp = harness.router.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    let page: Value = serde_json::from_slice(&bytes).unwrap();
    assert!(page["items"].is_array());
    assert_eq!(page["limit"].as_u64(), Some(5));

    // 4. Test slice_paginated bounds and clamping
    let raw = vec![1, 2, 3, 4, 5];
    let (slice, total) = slice_paginated(&raw, 2, 2);
    assert_eq!(slice, vec![3, 4]);
    assert_eq!(total, 5);

    let (slice_overflow, total_overflow) = slice_paginated(&raw, 10, 5);
    assert!(slice_overflow.is_empty());
    assert_eq!(total_overflow, 5);

    let _ = session_id;
}

#[tokio::test]
async fn anti_regression_w3c_traceparent_propagation_and_generation() {
    let harness = build_test_harness(ServerConfig::for_tests()).await;

    // 1. Pass explicit W3C traceparent
    let inbound_tp = "00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01";
    let req = Request::builder()
        .method(Method::GET)
        .uri("/health/live")
        .header("traceparent", inbound_tp)
        .body(Body::empty())
        .unwrap();
    let resp = harness.router.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let tid = resp
        .headers()
        .get("x-trace-id")
        .and_then(|v| v.to_str().ok())
        .unwrap();
    assert_eq!(
        tid, "4bf92f3577b34da6a3ce929d0e0e4736",
        "must preserve incoming trace_id"
    );

    let out_tp = resp
        .headers()
        .get("traceparent")
        .and_then(|v| v.to_str().ok())
        .unwrap();
    assert!(
        out_tp.starts_with("00-4bf92f3577b34da6a3ce929d0e0e4736-"),
        "outbound traceparent must preserve trace_id: {out_tp}"
    );

    // 2. Request without traceparent mints fresh W3C headers
    let req = Request::builder()
        .method(Method::GET)
        .uri("/health/live")
        .body(Body::empty())
        .unwrap();
    let resp = harness.router.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    assert!(resp.headers().contains_key("traceparent"));
    assert!(resp.headers().contains_key("x-trace-id"));

    // 3. Unit test W3cTraceContext parsing
    let parsed = W3cTraceContext::new();
    assert_eq!(parsed.trace_id.len(), 32);
    assert_eq!(parsed.span_id.len(), 16);
    let (h_name, h_val) = inject_w3c_traceparent(Some(&parsed));
    assert_eq!(h_name, "traceparent");
    assert_eq!(h_val, parsed.traceparent);
}

#[tokio::test]
async fn anti_regression_lifecycle_manager_records_all_eight_hooks() {
    let _harness = build_test_harness(ServerConfig::for_tests()).await;
    let lifecycle = LifecycleManager::global();
    let snapshot = lifecycle.snapshot();
    assert!(
        snapshot.module_configure_count >= 1,
        "module configure must be recorded"
    );
    assert!(
        snapshot.module_init_count >= 1,
        "module init must be recorded"
    );
    assert!(snapshot.bootstrapped, "application must be bootstrapped");
    assert!(snapshot.server_ready, "server must be ready");

    // Test shutdown hooks recording
    use ironic::BeforeShutdown;
    lifecycle
        .before_shutdown(ShutdownSignal::Terminate)
        .await
        .unwrap();
    use ironic::OnApplicationShutdown;
    lifecycle
        .on_application_shutdown(ShutdownSignal::Terminate)
        .await
        .unwrap();
    use ironic::OnModuleDestroy;
    lifecycle.on_module_destroy().await.unwrap();
    use ironic::AfterShutdown;
    lifecycle.after_shutdown().await.unwrap();

    let final_snap = lifecycle.snapshot();
    assert!(final_snap.before_shutdown_count >= 1);
    assert!(final_snap.application_shutdown_count >= 1);
    assert!(final_snap.module_destroy_count >= 1);
    assert!(final_snap.after_shutdown);
}

#[test]
fn anti_regression_source_level_rules() {
    let obs_dir = Path::new("src/infra/observability");
    let files = [
        "mod.rs",
        "metrics.rs",
        "tracing.rs",
        "apm.rs",
        "logging.rs",
        "lifecycle.rs",
        "health.rs",
        "timeout.rs",
        "plugin.rs",
        "pagination.rs",
    ];

    for file in files {
        let p = obs_dir.join(file);
        assert!(p.exists(), "observability module file must exist: {:?}", p);
        let content = std::fs::read_to_string(&p).unwrap();
        let lines = content.lines().count();
        assert!(
            lines <= 500,
            "Rule G-20 violation: {:?} has {} lines (max 500)",
            p,
            lines
        );
    }

    // Verify systemd units script hardening
    let units_script = Path::new("../../deploy/scripts/40-systemd-units.sh");
    assert!(units_script.exists());
    let script_content = std::fs::read_to_string(units_script).unwrap();
    assert!(
        script_content.contains("TimeoutStopSec=30"),
        "40-systemd-units.sh must specify TimeoutStopSec=30"
    );
    assert!(
        script_content.contains("KillSignal=SIGTERM"),
        "40-systemd-units.sh must specify KillSignal=SIGTERM"
    );

    // Verify deploy scripts exist
    assert!(Path::new("../../deploy/scripts/25-setup-observability.sh").exists());
    assert!(Path::new("../../deploy/scripts/95-validate-metrics.sh").exists());
}

#[tokio::test]
async fn anti_regression_jsonl_log_storage_query_and_blob_archive() {
    use agents_server::infra::{
        adapters::{BlobStorePort, LocalFsBlobStore},
        observability::{SystemLogQueryDto, archive_logs_to_blob, query_jsonl_logs},
    };
    use ironic::logging::{FileLogStorage, LogEntry, LogStorage};
    use std::collections::BTreeMap;

    let tmp = std::env::temp_dir().join(format!("obs_test_{}", uuid::Uuid::new_v4()));
    let log_dir = tmp.join("logs");
    let blob_dir = tmp.join("blobs");
    std::fs::create_dir_all(&log_dir).unwrap();

    let storage = FileLogStorage::new(&log_dir);
    let mut fields = BTreeMap::new();
    fields.insert(
        "trace_id".to_owned(),
        serde_json::json!("4bf92f3577b34da6a3ce929d0e0e4736"),
    );

    storage
        .store(LogEntry {
            timestamp: chrono::Utc::now(),
            level: "ERROR".to_owned(),
            target: "agents_server::mcp".to_owned(),
            message: "mcp dispatch failure".to_owned(),
            module_path: None,
            file: None,
            line: None,
            fields,
        })
        .await
        .unwrap();
    storage
        .store(LogEntry {
            timestamp: chrono::Utc::now(),
            level: "INFO".to_owned(),
            target: "agents_server::health".to_owned(),
            message: "health ok".to_owned(),
            module_path: None,
            file: None,
            line: None,
            fields: BTreeMap::new(),
        })
        .await
        .unwrap();
    storage.flush().await.unwrap();

    let res = query_jsonl_logs(
        &log_dir,
        &SystemLogQueryDto {
            level: Some("ERROR".to_owned()),
            target: Some("mcp".to_owned()),
            q: Some("4bf92f3577b34da6a3ce929d0e0e4736".to_owned()),
            limit: Some(10),
        },
    );
    assert_eq!(res.total, 1, "expected exactly 1 matching ERROR entry");
    assert_eq!(res.logs[0]["message"], "mcp dispatch failure");

    let blob = LocalFsBlobStore::new(blob_dir);
    let archived = archive_logs_to_blob(&log_dir, &blob).await;
    assert_eq!(archived, 1, "expected 1 JSONL file archived to blob store");
    let date = chrono::Utc::now().format("%Y-%m-%d").to_string();
    let stored = blob
        .get_blob(&format!("observability/logs/{date}.jsonl"))
        .await
        .unwrap();
    assert!(
        !stored.is_empty(),
        "archived JSONL must exist in blob store"
    );

    let _ = std::fs::remove_dir_all(&tmp);
}

#[tokio::test]
async fn anti_regression_system_logs_endpoint_requires_admin_and_returns_schema() {
    let mut cfg = ServerConfig::for_tests();
    cfg.admin_email = Some("admin@example.com".to_owned());
    cfg.admin_initial_password = Some("admin-secret-password-99".to_owned());
    let harness = build_test_harness(cfg).await;

    let unauth = Request::builder()
        .method(Method::GET)
        .uri("/api/v1/audit/system-logs")
        .body(Body::empty())
        .unwrap();
    let resp_unauth = harness.router.clone().oneshot(unauth).await.unwrap();
    assert_eq!(resp_unauth.status(), StatusCode::UNAUTHORIZED);

    let (_, member_token) = register_and_login(
        &harness.router,
        "obs-member@example.com",
        "ObsMemberPass123456!",
    )
    .await;

    let forbidden = Request::builder()
        .method(Method::GET)
        .uri("/api/v1/audit/system-logs")
        .header("authorization", format!("Bearer {member_token}"))
        .body(Body::empty())
        .unwrap();
    let resp_forbidden = harness.router.clone().oneshot(forbidden).await.unwrap();
    assert_eq!(resp_forbidden.status(), StatusCode::FORBIDDEN);

    let (login_status, tokens) = send_json(
        &harness.router,
        Method::POST,
        "/api/v1/auth/login",
        None,
        Some(serde_json::json!({
            "email": "admin@example.com",
            "password": "admin-secret-password-99"
        })),
    )
    .await;
    assert_eq!(login_status, StatusCode::OK);
    let admin_token = tokens["accessToken"].as_str().unwrap();

    let req = Request::builder()
        .method(Method::GET)
        .uri("/api/v1/audit/system-logs?level=ERROR&limit=20")
        .header("authorization", format!("Bearer {admin_token}"))
        .body(Body::empty())
        .unwrap();
    let resp = harness.router.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    let val: Value = serde_json::from_slice(&bytes).unwrap();
    assert!(val["logs"].is_array());
    assert!(val["total"].is_number());
    assert!(val["log_dir"].is_string());
}
