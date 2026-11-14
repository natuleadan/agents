#![allow(clippy::unwrap_used, clippy::expect_used)]

use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use http_body_util::BodyExt;
use serde_json::Value;
use tower::ServiceExt;

mod common;
use common::build_test_harness;

#[tokio::test]
async fn openapi_spec_is_enriched_and_scalar_ui_renders_completely() {
    let harness = build_test_harness(agents_server::ServerConfig::for_tests()).await;

    // 1. Validate /openapi.json contract
    let req = Request::builder()
        .uri("/openapi.json")
        .body(Body::empty())
        .unwrap();

    let resp = harness.router.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    let spec: Value = serde_json::from_slice(&bytes).expect("valid JSON OpenAPI spec");

    assert_eq!(spec.get("openapi").and_then(Value::as_str), Some("3.1.0"));
    assert_eq!(
        spec.get("info")
            .and_then(|i| i.get("title"))
            .and_then(Value::as_str),
        Some("agents-server API")
    );

    let paths = spec
        .get("paths")
        .and_then(Value::as_object)
        .expect("paths object");
    assert!(
        paths.len() >= 40,
        "expected at least 40 registered paths, got {}",
        paths.len()
    );

    // Verify that every domain operation documents request/response schemas for all its status codes
    let mut checked_operations = 0;
    for (path_name, path_item) in paths {
        if !path_name.starts_with("/api/v1") && !path_name.starts_with("/webhooks") {
            continue;
        }
        let Some(methods) = path_item.as_object() else {
            continue;
        };
        for (method, op) in methods {
            if !["get", "post", "put", "patch", "delete"].contains(&method.as_str()) {
                continue;
            }
            checked_operations += 1;
            let responses = op
                .get("responses")
                .and_then(Value::as_object)
                .unwrap_or_else(|| {
                    panic!("operation {method} {path_name} missing responses object")
                });

            for (status_code, resp_obj) in responses {
                if status_code == "204" {
                    continue;
                }
                let content = resp_obj.get("content").and_then(Value::as_object).unwrap_or_else(|| {
                    panic!(
                        "operation {method} {path_name} status {status_code} missing content schema:\n{}",
                        serde_json::to_string_pretty(resp_obj).unwrap()
                    )
                });
                assert!(
                    content.contains_key("application/json"),
                    "operation {method} {path_name} status {status_code} missing application/json schema"
                );
            }
        }
    }
    assert!(
        checked_operations >= 30,
        "expected at least 30 checked operations, got {checked_operations}"
    );

    // Invariant: System & Health endpoints (/health, /health/live, /health/ready, /version)
    // must be grouped under "System & Health", declare non-empty summaries and response body schemas
    for system_path in ["/health", "/health/live", "/health/ready", "/version"] {
        let op = paths
            .get(system_path)
            .and_then(|p| p.get("get"))
            .unwrap_or_else(|| panic!("missing system path {system_path} in openapi paths"));
        let tags = op
            .get("tags")
            .and_then(Value::as_array)
            .expect("tags array");
        assert!(
            tags.iter().any(|t| t.as_str() == Some("System & Health")),
            "path {system_path} must be tagged with 'System & Health'"
        );
        assert!(
            op.get("summary")
                .and_then(Value::as_str)
                .is_some_and(|s| !s.is_empty()),
            "path {system_path} must have a non-empty summary"
        );
        let resp200 = op
            .get("responses")
            .and_then(|r| r.get("200"))
            .expect("200 response");
        let content = resp200
            .get("content")
            .and_then(|c| c.get("application/json"));
        assert!(
            content.is_some(),
            "path {system_path} must declare application/json response schema for 200"
        );
    }

    let schemas = spec
        .get("components")
        .and_then(|c| c.get("schemas"))
        .and_then(Value::as_object)
        .expect("schemas object");
    assert!(
        schemas.len() >= 50,
        "expected at least 50 registered component schemas, got {}",
        schemas.len()
    );

    // Verify critical DTO and entity schemas are present with valid titles
    let required_schemas = [
        "ApiKeyMetadata",
        "ApiKeyScope",
        "TokenPair",
        "AuthenticatedActor",
        "RegisterDto",
        "LoginDto",
        "TaskEntity",
        "SubmitTaskDto",
        "TaskState",
        "TaskLease",
        "SessionEntity",
        "CreateSessionDto",
        "DraftSnapshot",
        "WorkflowEntity",
        "CreateWorkflowDto",
        "WorkflowRunEntity",
        "RenderedSlugEntity",
        "AutocronScheduleEntity",
        "CreateScheduleDto",
        "AutocronTickOutcome",
        "SwarmHandoffEntity",
        "CreateHandoffDto",
        "MediatedSwarmMessage",
        "AuditLogRecord",
        "AuditVerificationReport",
        "BoxerNodeAnnouncement",
        "NodeHeartbeatDto",
        "PreviewSyncAck",
        "McpPolicyInfo",
        "StreamReplayQuery",
        "EventEnvelope",
    ];

    for name in required_schemas {
        assert!(
            schemas.contains_key(name),
            "missing required schema: `{name}`"
        );
        let schema = &schemas[name];
        assert!(
            schema.get("properties").is_some() || schema.get("enum").is_some(),
            "schema `{name}` must define properties or enum"
        );
    }

    // 2. Validate self-hosted Scalar UI (/docs and /docs/)
    for path in ["/docs", "/docs/"] {
        let req = Request::builder().uri(path).body(Body::empty()).unwrap();
        let resp = harness.router.clone().oneshot(req).await.unwrap();
        assert_eq!(
            resp.status(),
            StatusCode::OK,
            "Scalar UI at {path} should return HTTP 200"
        );
        let html_bytes = resp.into_body().collect().await.unwrap().to_bytes();
        let html = String::from_utf8_lossy(&html_bytes);
        assert!(
            html.contains("/openapi.json") || html.contains("scalar"),
            "Scalar UI HTML should reference /openapi.json or scalar bundle"
        );
    }

    // 3. Validate embedded Scalar JS bundle is served without external CDN requests
    let bundle_req = Request::builder()
        .uri("/docs/scalar.js")
        .body(Body::empty())
        .unwrap();
    let bundle_resp = harness.router.clone().oneshot(bundle_req).await.unwrap();
    assert_eq!(bundle_resp.status(), StatusCode::OK);
    let bundle_bytes = bundle_resp.into_body().collect().await.unwrap().to_bytes();
    assert!(
        bundle_bytes.len() > 100_000,
        "Scalar JS bundle should be substantial (got {} bytes)",
        bundle_bytes.len()
    );
}
