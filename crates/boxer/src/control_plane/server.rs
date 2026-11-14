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
        openapi::boxer_openapi_spec,
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
        .route("/health", get(health_ready_handler))
        .route("/health/live", get(health_live_handler))
        .route("/health/ready", get(health_ready_handler))
        .route("/version", get(version_handler))
        .route("/metrics", get(boxer_metrics_handler))
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
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            boxer_observability_middleware,
        ))
        .layer(ironic::metrics::MetricsLayer::new(
            ironic::metrics::MetricsConfig::default(),
        ))
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

async fn boxer_metrics_handler(State(state): State<ControlPlaneState>) -> impl IntoResponse {
    let text = state.service.metrics_prometheus().await;
    (
        [(
            axum::http::header::CONTENT_TYPE,
            "text/plain; version=0.0.4; charset=utf-8",
        )],
        text,
    )
}

async fn boxer_observability_middleware(
    State(state): State<ControlPlaneState>,
    req: axum::extract::Request,
    next: axum::middleware::Next,
) -> Response {
    let start = std::time::Instant::now();
    let method = req.method().clone();
    let uri = req.uri().clone();

    let request_id = req
        .headers()
        .get("x-request-id")
        .and_then(|v| v.to_str().ok())
        .map(ToOwned::to_owned)
        .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());

    let traceparent = req
        .headers()
        .get("traceparent")
        .and_then(|v| v.to_str().ok())
        .map(ToOwned::to_owned)
        .unwrap_or_else(|| {
            format!(
                "00-{:032x}-{:016x}-01",
                uuid::Uuid::new_v4().as_u128(),
                (uuid::Uuid::new_v4().as_u128() >> 64) as u64
            )
        });

    let mut response = next.run(req).await;
    let status = response.status();
    let duration_ms = start.elapsed().as_millis();

    if status == StatusCode::UNAUTHORIZED || status == StatusCode::FORBIDDEN {
        state.service.record_auth_failure();
    }

    if let Ok(val) = axum::http::HeaderValue::from_str(&request_id) {
        response
            .headers_mut()
            .insert(axum::http::HeaderName::from_static("x-request-id"), val);
    }
    if let Ok(val) = axum::http::HeaderValue::from_str(&traceparent) {
        response
            .headers_mut()
            .insert(axum::http::HeaderName::from_static("traceparent"), val);
    }

    tracing::info!(
        target: "boxer.http.access",
        method = %method,
        uri = %uri,
        status = %status.as_u16(),
        duration_ms,
        traceparent = %traceparent,
        "boxer http access"
    );

    response
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
        Some(token)
            if bool::from(subtle::ConstantTimeEq::ct_eq(
                token.as_bytes(),
                secret.as_bytes(),
            )) =>
        {
            Ok(())
        }
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

async fn version_handler() -> impl IntoResponse {
    (
        StatusCode::OK,
        Json(serde_json::json!({
            "service": "agents-boxer",
            "version": env!("CARGO_PKG_VERSION")
        })),
    )
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
