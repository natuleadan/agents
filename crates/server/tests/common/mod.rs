#![allow(dead_code, clippy::unwrap_used, clippy::expect_used)]

use std::sync::Arc;

use agents_server::{
    InfrastructureHandles, ServerConfig, build_application,
    infra::{
        admission::AdmissionController,
        bus::{InMemoryMessageBus, MessageBus},
        state::{InMemoryStateBackend, StateBackend},
        stream::{InMemoryStreamRepository, StreamStore},
    },
    modules::{
        audit::repositories::{AuditRepository, AuditStore, InMemoryAuditRepository},
        auth::repositories::{AuthRepository, AuthStore, InMemoryAuthRepository},
        sessions::repositories::{InMemorySessionRepository, SessionStore},
        tasks::repositories::{InMemoryTaskRepository, TaskRepository, TaskStore},
    },
};
use axum::{
    Router,
    body::Body,
    http::{Method, Request, StatusCode},
};
use http_body_util::BodyExt;
use ironic::{Application, OpenApiAxumApplication};
use serde_json::{Value, json};
use tower::ServiceExt;

pub struct TestHarness {
    pub app: Application<OpenApiAxumApplication>,
    pub router: Router,
    pub config: ServerConfig,
    pub bus: Arc<InMemoryMessageBus>,
    pub state: Arc<InMemoryStateBackend>,
    pub auth_repo: Arc<InMemoryAuthRepository>,
    pub audit_repo: Arc<InMemoryAuditRepository>,
    pub task_repo: Arc<InMemoryTaskRepository>,
    pub admission: Arc<AdmissionController>,
}

pub async fn build_test_harness(mut config: ServerConfig) -> TestHarness {
    config.rate_limit_max_requests = 1000;
    build_test_harness_with_config(config).await
}

pub async fn build_test_harness_with_config(config: ServerConfig) -> TestHarness {
    let state = InMemoryStateBackend::new();
    let bus = InMemoryMessageBus::new();
    let admission =
        AdmissionController::new(config.max_concurrent_global, config.max_concurrent_session);
    let audit_repo = InMemoryAuditRepository::new();
    let auth_repo = InMemoryAuthRepository::new();
    let session_repo = InMemorySessionRepository::new();
    let task_repo = InMemoryTaskRepository::new();
    let stream_repo = InMemoryStreamRepository::new();

    let infra = InfrastructureHandles {
        config: config.clone(),
        database: None,
        state: Arc::clone(&state) as Arc<dyn StateBackend>,
        bus: Arc::clone(&bus) as Arc<dyn MessageBus>,
        admission: Arc::clone(&admission),
        audit_store: AuditStore(Arc::clone(&audit_repo) as Arc<dyn AuditRepository>),
        auth_store: AuthStore(Arc::clone(&auth_repo) as Arc<dyn AuthRepository>),
        session_store: SessionStore(session_repo),
        task_store: TaskStore(Arc::clone(&task_repo) as Arc<dyn TaskRepository>),
        stream_store: StreamStore(stream_repo),
    };

    let app = build_application(infra)
        .await
        .expect("test application must build cleanly");
    let router = app.platform().router().clone();

    TestHarness {
        app,
        router,
        config,
        bus,
        state,
        auth_repo,
        audit_repo,
        task_repo,
        admission,
    }
}

pub async fn send_json(
    router: &Router,
    method: Method,
    uri: &str,
    bearer: Option<&str>,
    body: Option<Value>,
) -> (StatusCode, Value) {
    send_json_with_headers(router, method, uri, bearer, &[], body).await
}

pub async fn send_json_with_headers(
    router: &Router,
    method: Method,
    uri: &str,
    bearer: Option<&str>,
    extra_headers: &[(&str, &str)],
    body: Option<Value>,
) -> (StatusCode, Value) {
    let mut builder = Request::builder().method(method).uri(uri);
    if let Some(tok) = bearer {
        builder = builder.header("authorization", format!("Bearer {tok}"));
    }
    for (k, v) in extra_headers {
        builder = builder.header(*k, *v);
    }
    let req = match body {
        Some(val) => builder
            .header("content-type", "application/json")
            .body(Body::from(serde_json::to_vec(&val).unwrap()))
            .unwrap(),
        None => builder.body(Body::empty()).unwrap(),
    };
    let resp = router.clone().oneshot(req).await.unwrap();
    let status = resp.status();
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    let parsed = if bytes.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&bytes)
            .unwrap_or_else(|_| json!({ "raw": String::from_utf8_lossy(&bytes).to_string() }))
    };
    (status, parsed)
}

pub async fn register_and_login(router: &Router, email: &str, password: &str) -> (String, String) {
    let (reg_status, reg_body) = send_json(
        router,
        Method::POST,
        "/api/v1/auth/register",
        None,
        Some(json!({ "email": email, "password": password })),
    )
    .await;
    assert_eq!(reg_status, StatusCode::OK, "register failed: {reg_body}");
    let user_id = reg_body["userId"].as_str().unwrap().to_owned();

    let (login_status, login_body) = send_json(
        router,
        Method::POST,
        "/api/v1/auth/login",
        None,
        Some(json!({ "email": email, "password": password })),
    )
    .await;
    assert_eq!(login_status, StatusCode::OK, "login failed: {login_body}");
    let token = login_body["accessToken"].as_str().unwrap().to_owned();
    (user_id, token)
}
