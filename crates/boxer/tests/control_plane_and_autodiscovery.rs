//! Anti-regression test suite validating Boxer HTTP control plane API (/v2/box),
//! RFC 7807 problem details error envelopes, cluster secret authentication, VFS confinement,
//! and native Ironic task scheduling for node autodiscovery heartbeats.

#![forbid(unsafe_code)]
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::{collections::HashMap, sync::Arc, time::Duration};

use agents_boxer::{
    config::{BoxerCli, BoxerConfig},
    control_plane::{
        BoxerControlPlaneService, build_control_plane_router,
        dto::{
            BoxStatusResponse, BoxerProblemDetails, ExecBoxRequest, ExecBoxResponse,
            ProvisionBoxRequest, ProvisionBoxResponse, ReadFileResponse, TeardownBoxResponse,
            WriteFileRequest,
        },
        start_autodiscovery_heartbeat,
    },
};
use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use tower::ServiceExt;

fn test_config() -> (Arc<BoxerConfig>, tempfile::TempDir) {
    let tmp = tempfile::tempdir().expect("tempdir");
    let cli = BoxerCli {
        node_id: "test-boxer-01".into(),
        setup_mode: "local".into(),
        runtime: "host".into(),
        isolation: "none".into(),
        allow_host_native: true,
        openshell_gateway_url: None,
        sandbox_image: "ghcr.io/natuleadan/agents-sandbox:latest".into(),
        workspace_root: tmp.path().to_path_buf(),
        max_concurrent_boxes: 4,
        nats_url: "nats://127.0.0.1:4222".into(),
        nats_tls_enabled: false,
        nats_nkey_seed: None,
        nats_jwt: None,
        server_internal_url: "http://127.0.0.1:8080".into(),
        worker_token: "test-worker-token".into(),
        proxy_bind_addr: "127.0.0.1:0".parse().unwrap(),
        private_vlan_only: false,
        traefik_preview_enabled: false,
        traefik_preview_domain: "preview.test".into(),
        traefik_dynamic_dir: tmp.path().join("traefik"),
        traefik_default_ttl_secs: 60,
        upstash_box_token: None,
        deepinfra_token: None,
        http_port: 9091,
        cluster_secret: "super-secret-cluster-token".into(),
    };
    let cfg = Arc::new(BoxerConfig::from_cli(cli).expect("valid config"));
    (cfg, tmp)
}

#[tokio::test]
async fn anti_regression_boxer_control_plane_crud_and_vfs() {
    let (cfg, _tmp) = test_config();
    let service = Arc::new(BoxerControlPlaneService::new(Arc::clone(&cfg)));
    let app = build_control_plane_router(Arc::clone(&service), &cfg);

    // 1. Health check & OpenAPI / Scalar docs
    let req = Request::builder()
        .uri("/health/ready")
        .body(Body::empty())
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let doc_req = Request::builder()
        .uri("/openapi.json")
        .body(Body::empty())
        .unwrap();
    let doc_resp = app.clone().oneshot(doc_req).await.unwrap();
    assert_eq!(doc_resp.status(), StatusCode::OK);
    let doc_bytes = axum::body::to_bytes(doc_resp.into_body(), 1024 * 64)
        .await
        .unwrap();
    let spec: serde_json::Value = serde_json::from_slice(&doc_bytes).unwrap();
    assert_eq!(spec["info"]["title"], "agents-boxer API");
    assert!(spec["paths"]["/v1/box"].is_object());

    let scalar_req = Request::builder().uri("/docs").body(Body::empty()).unwrap();
    let scalar_resp = app.clone().oneshot(scalar_req).await.unwrap();
    assert_eq!(scalar_resp.status(), StatusCode::OK);

    // 2. Provision sandbox via /v1/box
    let prov_body = serde_json::to_vec(&ProvisionBoxRequest {
        session_id: "sess-p15".into(),
        task_id: "task-p15".into(),
        profile: None,
        env: HashMap::new(),
    })
    .unwrap();

    let req = Request::builder()
        .method("POST")
        .uri("/v1/box")
        .header("Authorization", "Bearer super-secret-cluster-token")
        .header("Content-Type", "application/json")
        .body(Body::from(prov_body))
        .unwrap();

    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::CREATED);
    let bytes = axum::body::to_bytes(resp.into_body(), 1024 * 64)
        .await
        .unwrap();
    let prov: ProvisionBoxResponse = serde_json::from_slice(&bytes).unwrap();
    assert!(prov.box_id.starts_with("box-"));
    assert_eq!(prov.status, "ready");

    // 3. Write file into sandbox VFS
    let write_body = serde_json::to_vec(&WriteFileRequest {
        path: "src/main.txt".into(),
        content: "hello phase 15".into(),
    })
    .unwrap();

    let req = Request::builder()
        .method("POST")
        .uri(format!("/v1/box/{}/files", prov.box_id))
        .header("Authorization", "Bearer super-secret-cluster-token")
        .header("Content-Type", "application/json")
        .body(Body::from(write_body))
        .unwrap();

    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    // 4. Read file back from sandbox VFS
    let req = Request::builder()
        .method("GET")
        .uri(format!("/v1/box/{}/files?path=src/main.txt", prov.box_id))
        .header("Authorization", "Bearer super-secret-cluster-token")
        .body(Body::empty())
        .unwrap();

    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let bytes = axum::body::to_bytes(resp.into_body(), 1024 * 64)
        .await
        .unwrap();
    let read_file: ReadFileResponse = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(read_file.content, "hello phase 15");

    // 5. Invariant: Path traversal must be blocked
    let req = Request::builder()
        .method("GET")
        .uri(format!(
            "/v1/box/{}/files?path=../../etc/passwd",
            prov.box_id
        ))
        .header("Authorization", "Bearer super-secret-cluster-token")
        .header("Content-Type", "application/json")
        .body(Body::empty())
        .unwrap();

    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);

    // 6. Execute hermetic command inside sandbox
    let exec_body = serde_json::to_vec(&ExecBoxRequest {
        command: "echo 'phase-15-ok'".into(),
        timeout_ms: Some(5000),
    })
    .unwrap();

    let req = Request::builder()
        .method("POST")
        .uri(format!("/v1/box/{}/exec", prov.box_id))
        .header("Authorization", "Bearer super-secret-cluster-token")
        .header("Content-Type", "application/json")
        .body(Body::from(exec_body))
        .unwrap();

    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let bytes = axum::body::to_bytes(resp.into_body(), 1024 * 64)
        .await
        .unwrap();
    let exec_res: ExecBoxResponse = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(exec_res.exit_code, 0);
    assert!(exec_res.stdout.contains("phase-15-ok"));

    // 7. Read sandbox status
    let req = Request::builder()
        .method("GET")
        .uri(format!("/v1/box/{}/status", prov.box_id))
        .header("Authorization", "Bearer super-secret-cluster-token")
        .body(Body::empty())
        .unwrap();

    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let bytes = axum::body::to_bytes(resp.into_body(), 1024 * 64)
        .await
        .unwrap();
    let status_res: BoxStatusResponse = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(status_res.status, "active");

    // 8. Teardown sandbox
    let req = Request::builder()
        .method("DELETE")
        .uri(format!("/v1/box/{}", prov.box_id))
        .header("Authorization", "Bearer super-secret-cluster-token")
        .body(Body::empty())
        .unwrap();

    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let bytes = axum::body::to_bytes(resp.into_body(), 1024 * 64)
        .await
        .unwrap();
    let teardown: TeardownBoxResponse = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(teardown.status, "destroyed");

    // 9. Backward compatibility alias: /v2/box works identically
    let req_v2 = Request::builder()
        .method("POST")
        .uri("/v2/box")
        .header("Authorization", "Bearer super-secret-cluster-token")
        .header("Content-Type", "application/json")
        .body(Body::from(
            serde_json::to_vec(&ProvisionBoxRequest {
                session_id: "sess-v2-alias".into(),
                task_id: "task-v2-alias".into(),
                profile: None,
                env: HashMap::new(),
            })
            .unwrap(),
        ))
        .unwrap();
    let resp_v2 = app.clone().oneshot(req_v2).await.unwrap();
    assert_eq!(resp_v2.status(), StatusCode::CREATED);
}

#[tokio::test]
async fn anti_regression_boxer_control_plane_auth_and_rfc7807() {
    let (cfg, _tmp) = test_config();
    let service = Arc::new(BoxerControlPlaneService::new(Arc::clone(&cfg)));
    let app = build_control_plane_router(Arc::clone(&service), &cfg);

    let req_body = serde_json::to_vec(&ProvisionBoxRequest {
        session_id: "s-auth".into(),
        task_id: "t-auth".into(),
        profile: None,
        env: HashMap::new(),
    })
    .unwrap();

    // Invariant 1: Missing auth header returns 401 Unauthorized with RFC 7807 problem details
    let req = Request::builder()
        .method("POST")
        .uri("/v1/box")
        .header("Content-Type", "application/json")
        .body(Body::from(req_body.clone()))
        .unwrap();

    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    let bytes = axum::body::to_bytes(resp.into_body(), 1024 * 64)
        .await
        .unwrap();
    let problem: BoxerProblemDetails = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(problem.status, 401);
    assert_eq!(problem.title, "Unauthorized");

    // Invariant 2: Invalid cluster secret returns 403 Forbidden with RFC 7807 problem details
    let req = Request::builder()
        .method("POST")
        .uri("/v2/box")
        .header("Authorization", "Bearer wrong-secret")
        .header("Content-Type", "application/json")
        .body(Body::from(req_body))
        .unwrap();

    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);
    let bytes = axum::body::to_bytes(resp.into_body(), 1024 * 64)
        .await
        .unwrap();
    let problem: BoxerProblemDetails = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(problem.status, 403);
    assert_eq!(problem.title, "Forbidden");
}

#[tokio::test]
async fn anti_regression_boxer_autodiscovery_heartbeat_task() {
    let (cfg, _tmp) = test_config();
    let service = Arc::new(BoxerControlPlaneService::new(Arc::clone(&cfg)));

    // Invariant: start_autodiscovery_heartbeat must run through native ironic interval
    let task = start_autodiscovery_heartbeat(
        Arc::clone(&cfg),
        Arc::clone(&service),
        Duration::from_millis(50),
    );

    task.pause();
    task.resume();

    let shutdown_res = task.shutdown().await;
    assert!(
        shutdown_res.is_ok(),
        "heartbeat scheduled task must shut down cleanly"
    );
}

#[tokio::test]
async fn anti_regression_boxer_health_version_and_openapi_schemas() {
    let (cfg, _tmp) = test_config();
    let service = Arc::new(BoxerControlPlaneService::new(Arc::clone(&cfg)));
    let app = build_control_plane_router(Arc::clone(&service), &cfg);

    for path in ["/health", "/health/live", "/health/ready", "/version"] {
        let req = Request::builder()
            .method("GET")
            .uri(path)
            .body(Body::empty())
            .unwrap();
        let resp = app.clone().oneshot(req).await.unwrap();
        assert_eq!(
            resp.status(),
            StatusCode::OK,
            "route {path} must return 200"
        );
    }

    let req = Request::builder()
        .method("GET")
        .uri("/openapi.json")
        .body(Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let bytes = axum::body::to_bytes(resp.into_body(), 1024 * 256)
        .await
        .unwrap();
    let spec: serde_json::Value = serde_json::from_slice(&bytes).unwrap();

    for schema_name in [
        "ProvisionBoxRequest",
        "ProvisionBoxResponse",
        "ExecBoxRequest",
        "ExecBoxResponse",
        "BoxStatusResponse",
        "WriteFileRequest",
        "WriteFileResponse",
        "ReadFileResponse",
        "TeardownBoxResponse",
        "BoxerHealthResponse",
        "VersionResponse",
        "BoxerProblemDetails",
    ] {
        assert!(
            spec["components"]["schemas"][schema_name].is_object(),
            "Boxer OpenAPI components.schemas.{schema_name} missing"
        );
    }

    assert!(
        spec["paths"]["/v1/box"]["post"]["requestBody"]["content"]["application/json"]["schema"]["$ref"]
            .as_str()
            .is_some(),
        "POST /v1/box must declare typed JSON requestBody schema"
    );
    assert!(
        spec["paths"]["/v1/box"]["post"]["responses"]["201"]["content"]["application/json"]["schema"]["$ref"]
            .as_str()
            .is_some(),
        "POST /v1/box 201 must declare typed JSON response schema"
    );
    assert!(
        spec["paths"]["/version"]["get"].is_object(),
        "/version must be documented in Boxer OpenAPI"
    );
}

#[tokio::test]
async fn anti_regression_boxer_signed_autodiscovery_heartbeat_delivery() {
    use axum::{Router, extract::State, response::IntoResponse, routing::post};
    use tokio::sync::{Mutex, Notify};

    #[derive(Clone, Default)]
    struct CaptureState {
        payload: Arc<Mutex<Option<serde_json::Value>>>,
        notify: Arc<Notify>,
    }

    async fn mock_register(
        State(state): State<CaptureState>,
        axum::Json(body): axum::Json<serde_json::Value>,
    ) -> impl IntoResponse {
        *state.payload.lock().await = Some(body);
        state.notify.notify_one();
        (
            StatusCode::OK,
            axum::Json(serde_json::json!({ "status": "registered" })),
        )
    }

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let capture = CaptureState::default();
    let mock_app = Router::new()
        .route("/api/v1/nodes/register", post(mock_register))
        .with_state(capture.clone());
    let server_handle = tokio::spawn(async move {
        let _ = axum::serve(listener, mock_app).await;
    });

    let (base_cfg, _tmp) = test_config();
    let mut cfg_val = (*base_cfg).clone();
    cfg_val.server_internal_url = format!("http://{addr}");
    let cfg = Arc::new(cfg_val);
    let service = Arc::new(BoxerControlPlaneService::new(Arc::clone(&cfg)));

    let task = start_autodiscovery_heartbeat(cfg, service, Duration::from_millis(25));
    tokio::time::timeout(Duration::from_secs(3), capture.notify.notified())
        .await
        .expect("signed heartbeat must arrive at /api/v1/nodes/register");
    let _ = task.shutdown().await;
    server_handle.abort();

    let payload = capture
        .payload
        .lock()
        .await
        .clone()
        .expect("captured payload");
    assert_eq!(payload["nodeId"], "test-boxer-01");
    assert_eq!(payload["nodeIp"], "127.0.0.1");
    assert_eq!(payload["port"], 9091);
    assert!(payload["timestampMs"].as_u64().unwrap_or(0) > 0);
    assert_eq!(
        payload["signature"].as_str().map(str::len),
        Some(64),
        "signed registration payload must contain 64-char HMAC-SHA256 hex digest"
    );
}
