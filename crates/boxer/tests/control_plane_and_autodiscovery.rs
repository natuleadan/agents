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

    // 1. Health check
    let req = Request::builder()
        .uri("/health/ready")
        .body(Body::empty())
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    // 2. Provision sandbox
    let prov_body = serde_json::to_vec(&ProvisionBoxRequest {
        session_id: "sess-p15".into(),
        task_id: "task-p15".into(),
        profile: None,
        env: HashMap::new(),
    })
    .unwrap();

    let req = Request::builder()
        .method("POST")
        .uri("/v2/box")
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
        .uri(format!("/v2/box/{}/files", prov.box_id))
        .header("Authorization", "Bearer super-secret-cluster-token")
        .header("Content-Type", "application/json")
        .body(Body::from(write_body))
        .unwrap();

    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    // 4. Read file back from sandbox VFS
    let req = Request::builder()
        .method("GET")
        .uri(format!("/v2/box/{}/files?path=src/main.txt", prov.box_id))
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
            "/v2/box/{}/files?path=../../etc/passwd",
            prov.box_id
        ))
        .header("Authorization", "Bearer super-secret-cluster-token")
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
        .uri(format!("/v2/box/{}/exec", prov.box_id))
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
        .uri(format!("/v2/box/{}/status", prov.box_id))
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
        .uri(format!("/v2/box/{}", prov.box_id))
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
        .uri("/v2/box")
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
