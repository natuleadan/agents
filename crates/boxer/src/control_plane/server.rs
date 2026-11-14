//! HTTP control plane routing and RFC 7807 problem details error mapping for Boxer.

use std::{collections::HashMap, sync::Arc};

use axum::{
    Router,
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Json, Response},
    routing::{delete, get, post},
};

use crate::{
    config::BoxerConfig,
    control_plane::{
        dto::{BoxerProblemDetails, ExecBoxRequest, ProvisionBoxRequest, WriteFileRequest},
        service::BoxerControlPlaneService,
    },
};

#[derive(Clone)]
pub struct ControlPlaneState {
    pub service: Arc<BoxerControlPlaneService>,
    pub cluster_secret: String,
}

/// Constructs the Axum/Ironic HTTP router for Boxer's `/v1/box` control plane.
pub fn build_control_plane_router(
    service: Arc<BoxerControlPlaneService>,
    config: &BoxerConfig,
) -> Router {
    let state = ControlPlaneState {
        service,
        cluster_secret: config.cluster_secret.clone(),
    };

    let scalar_cfg = serde_json::json!({
        "url": "/openapi.json",
        "theme": "purple",
        "hideModels": false,
        "showSidebar": true
    });
    let scalar_cfg_slash = scalar_cfg.clone();

    let api_router = Router::new()
        .route("/health/live", get(health_live_handler))
        .route("/health/ready", get(health_ready_handler))
        // v1 routes (primary)
        .route("/v1/box", post(provision_box_handler))
        .route("/v1/box/{id}/exec", post(exec_box_handler))
        .route("/v1/box/{id}/status", get(status_box_handler))
        .route(
            "/v1/box/{id}/files",
            get(read_file_handler).post(write_file_handler),
        )
        .route("/v1/box/{id}", delete(teardown_box_handler))
        // v2 routes (backward-compatibility alias)
        .route("/v2/box", post(provision_box_handler))
        .route("/v2/box/{id}/exec", post(exec_box_handler))
        .route("/v2/box/{id}/status", get(status_box_handler))
        .route(
            "/v2/box/{id}/files",
            get(read_file_handler).post(write_file_handler),
        )
        .route("/v2/box/{id}", delete(teardown_box_handler))
        .with_state(state);

    api_router
        .route("/openapi.json", get(boxer_openapi_handler))
        .merge(scalar_api_reference::axum::router("/docs", &scalar_cfg))
        .route(
            "/docs/",
            get(move || async move {
                scalar_api_reference::axum::scalar_response(
                    &scalar_cfg_slash,
                    Some("/docs/scalar.js"),
                )
            }),
        )
}

async fn boxer_openapi_handler() -> impl IntoResponse {
    let spec = boxer_openapi_spec();
    (
        [(axum::http::header::CONTENT_TYPE, "application/json")],
        spec.to_string(),
    )
}

fn boxer_openapi_spec() -> serde_json::Value {
    serde_json::json!({
        "openapi": "3.1.0",
        "info": {
            "title": "agents-boxer API",
            "version": env!("CARGO_PKG_VERSION"),
            "description": "Internal sandbox supervisor and execution daemon control plane"
        },
        "tags": [
            { "name": "System & Health", "description": "Liveness and readiness probes" },
            { "name": "Sandboxes", "description": "Confined sandbox provisioning, execution, and VFS file operations" }
        ],
        "paths": {
            "/health/live": {
                "get": {
                    "tags": ["System & Health"],
                    "summary": "Liveness probe",
                    "description": "Verifies that the Boxer daemon process is active.",
                    "responses": {
                        "200": { "description": "Daemon is alive" }
                    }
                }
            },
            "/health/ready": {
                "get": {
                    "tags": ["System & Health"],
                    "summary": "Readiness probe",
                    "description": "Returns current node capacity and active box counts.",
                    "responses": {
                        "200": { "description": "Boxer node ready to accept sandboxes" }
                    }
                }
            },
            "/v1/box": {
                "post": {
                    "tags": ["Sandboxes"],
                    "summary": "Provision a new sandbox",
                    "description": "Allocates an isolated workspace directory and provisions container runtime resources.",
                    "security": [{ "clusterSecretAuth": [] }],
                    "responses": {
                        "201": { "description": "Sandbox provisioned successfully" },
                        "401": { "description": "Missing or invalid cluster secret" },
                        "429": { "description": "Capacity exceeded" },
                        "500": { "description": "Provisioning failure" }
                    }
                }
            },
            "/v1/box/{id}/exec": {
                "post": {
                    "tags": ["Sandboxes"],
                    "summary": "Execute command in sandbox",
                    "description": "Runs a command hermetically inside the sandbox with stdout/stderr capture and cancellation guards.",
                    "security": [{ "clusterSecretAuth": [] }],
                    "responses": {
                        "200": { "description": "Execution completed" },
                        "401": { "description": "Missing or invalid cluster secret" },
                        "404": { "description": "Sandbox not found" },
                        "500": { "description": "Execution failure" }
                    }
                }
            },
            "/v1/box/{id}/status": {
                "get": {
                    "tags": ["Sandboxes"],
                    "summary": "Get sandbox status and metrics",
                    "security": [{ "clusterSecretAuth": [] }],
                    "responses": {
                        "200": { "description": "Current sandbox runtime metrics" },
                        "404": { "description": "Sandbox not found" }
                    }
                }
            },
            "/v1/box/{id}/files": {
                "get": {
                    "tags": ["Sandboxes"],
                    "summary": "Read file from sandbox VFS",
                    "security": [{ "clusterSecretAuth": [] }],
                    "responses": {
                        "200": { "description": "File contents read successfully" },
                        "400": { "description": "Path traversal attempt blocked" },
                        "404": { "description": "File not found" }
                    }
                },
                "post": {
                    "tags": ["Sandboxes"],
                    "summary": "Write file to sandbox VFS",
                    "security": [{ "clusterSecretAuth": [] }],
                    "responses": {
                        "200": { "description": "File written successfully" },
                        "400": { "description": "Path traversal attempt blocked" },
                        "404": { "description": "Sandbox not found" }
                    }
                }
            },
            "/v1/box/{id}": {
                "delete": {
                    "tags": ["Sandboxes"],
                    "summary": "Teardown and purge sandbox",
                    "security": [{ "clusterSecretAuth": [] }],
                    "responses": {
                        "200": { "description": "Sandbox teardown complete" },
                        "404": { "description": "Sandbox not found" }
                    }
                }
            }
        },
        "components": {
            "securitySchemes": {
                "clusterSecretAuth": {
                    "type": "http",
                    "scheme": "bearer",
                    "description": "Boxer cluster secret passed via Authorization Bearer header or X-Boxer-Secret"
                }
            }
        }
    })
}

fn verify_auth(
    headers: &HeaderMap,
    secret: &str,
) -> Result<(), (StatusCode, Json<BoxerProblemDetails>)> {
    let auth = headers
        .get("authorization")
        .and_then(|h| h.to_str().ok())
        .and_then(|s| s.strip_prefix("Bearer "))
        .or_else(|| headers.get("x-boxer-secret").and_then(|h| h.to_str().ok()));

    match auth {
        Some(token) if token == secret => Ok(()),
        Some(_) => Err((
            StatusCode::FORBIDDEN,
            Json(BoxerProblemDetails {
                error_type: "https://tools.ietf.org/html/rfc7807".into(),
                title: "Forbidden".into(),
                status: StatusCode::FORBIDDEN.as_u16(),
                detail: "invalid cluster secret".into(),
            }),
        )),
        None => Err((
            StatusCode::UNAUTHORIZED,
            Json(BoxerProblemDetails {
                error_type: "https://tools.ietf.org/html/rfc7807".into(),
                title: "Unauthorized".into(),
                status: StatusCode::UNAUTHORIZED.as_u16(),
                detail: "missing authorization bearer token".into(),
            }),
        )),
    }
}

async fn health_live_handler() -> impl IntoResponse {
    (
        StatusCode::OK,
        Json(serde_json::json!({ "status": "alive" })),
    )
}

async fn health_ready_handler(State(state): State<ControlPlaneState>) -> impl IntoResponse {
    let report = state.service.health().await;
    (StatusCode::OK, Json(report))
}

async fn provision_box_handler(
    State(state): State<ControlPlaneState>,
    headers: HeaderMap,
    Json(payload): Json<ProvisionBoxRequest>,
) -> Response {
    if let Err(err) = verify_auth(&headers, &state.cluster_secret) {
        return err.into_response();
    }

    match state.service.provision(payload).await {
        Ok(res) => (StatusCode::CREATED, Json(res)).into_response(),
        Err(err) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(BoxerProblemDetails {
                error_type: "https://tools.ietf.org/html/rfc7807".into(),
                title: "Provision Error".into(),
                status: 500,
                detail: err.to_string(),
            }),
        )
            .into_response(),
    }
}

async fn exec_box_handler(
    State(state): State<ControlPlaneState>,
    Path(id): Path<String>,
    headers: HeaderMap,
    Json(payload): Json<ExecBoxRequest>,
) -> Response {
    if let Err(err) = verify_auth(&headers, &state.cluster_secret) {
        return err.into_response();
    }

    match state.service.exec(&id, payload).await {
        Ok(res) => (StatusCode::OK, Json(res)).into_response(),
        Err(err) => (
            StatusCode::NOT_FOUND,
            Json(BoxerProblemDetails {
                error_type: "https://tools.ietf.org/html/rfc7807".into(),
                title: "Execution Error".into(),
                status: 404,
                detail: err.to_string(),
            }),
        )
            .into_response(),
    }
}

async fn status_box_handler(
    State(state): State<ControlPlaneState>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> Response {
    if let Err(err) = verify_auth(&headers, &state.cluster_secret) {
        return err.into_response();
    }

    match state.service.status(&id).await {
        Ok(res) => (StatusCode::OK, Json(res)).into_response(),
        Err(err) => (
            StatusCode::NOT_FOUND,
            Json(BoxerProblemDetails {
                error_type: "https://tools.ietf.org/html/rfc7807".into(),
                title: "Box Not Found".into(),
                status: 404,
                detail: err.to_string(),
            }),
        )
            .into_response(),
    }
}

async fn read_file_handler(
    State(state): State<ControlPlaneState>,
    Path(id): Path<String>,
    Query(params): Query<HashMap<String, String>>,
    headers: HeaderMap,
) -> Response {
    if let Err(err) = verify_auth(&headers, &state.cluster_secret) {
        return err.into_response();
    }

    let Some(path) = params.get("path") else {
        return (
            StatusCode::BAD_REQUEST,
            Json(BoxerProblemDetails {
                error_type: "https://tools.ietf.org/html/rfc7807".into(),
                title: "Bad Request".into(),
                status: 400,
                detail: "missing `path` query parameter".into(),
            }),
        )
            .into_response();
    };

    match state.service.read_file(&id, path).await {
        Ok(res) => (StatusCode::OK, Json(res)).into_response(),
        Err(err) => (
            StatusCode::NOT_FOUND,
            Json(BoxerProblemDetails {
                error_type: "https://tools.ietf.org/html/rfc7807".into(),
                title: "File Not Found".into(),
                status: 404,
                detail: err.to_string(),
            }),
        )
            .into_response(),
    }
}

async fn write_file_handler(
    State(state): State<ControlPlaneState>,
    Path(id): Path<String>,
    headers: HeaderMap,
    Json(payload): Json<WriteFileRequest>,
) -> Response {
    if let Err(err) = verify_auth(&headers, &state.cluster_secret) {
        return err.into_response();
    }

    match state.service.write_file(&id, payload).await {
        Ok(()) => (
            StatusCode::OK,
            Json(serde_json::json!({ "status": "written" })),
        )
            .into_response(),
        Err(err) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(BoxerProblemDetails {
                error_type: "https://tools.ietf.org/html/rfc7807".into(),
                title: "Write Error".into(),
                status: 500,
                detail: err.to_string(),
            }),
        )
            .into_response(),
    }
}

async fn teardown_box_handler(
    State(state): State<ControlPlaneState>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> Response {
    if let Err(err) = verify_auth(&headers, &state.cluster_secret) {
        return err.into_response();
    }

    match state.service.teardown(&id).await {
        Ok(res) => (StatusCode::OK, Json(res)).into_response(),
        Err(err) => (
            StatusCode::NOT_FOUND,
            Json(BoxerProblemDetails {
                error_type: "https://tools.ietf.org/html/rfc7807".into(),
                title: "Teardown Error".into(),
                status: 404,
                detail: err.to_string(),
            }),
        )
            .into_response(),
    }
}
