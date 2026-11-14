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

/// Constructs the Axum/Ironic HTTP router for Boxer's `/v2/box` control plane.
pub fn build_control_plane_router(
    service: Arc<BoxerControlPlaneService>,
    config: &BoxerConfig,
) -> Router {
    let state = ControlPlaneState {
        service,
        cluster_secret: config.cluster_secret.clone(),
    };

    Router::new()
        .route("/health/live", get(health_live_handler))
        .route("/health/ready", get(health_ready_handler))
        .route("/v2/box", post(provision_box_handler))
        .route("/v2/box/{id}/exec", post(exec_box_handler))
        .route("/v2/box/{id}/status", get(status_box_handler))
        .route(
            "/v2/box/{id}/files",
            get(read_file_handler).post(write_file_handler),
        )
        .route("/v2/box/{id}", delete(teardown_box_handler))
        .with_state(state)
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
