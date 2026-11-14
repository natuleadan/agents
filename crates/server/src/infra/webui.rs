//! Static Web UI asset routes backed cleanly by [`agents_styles`] token & bundle generators.

use std::sync::{Arc, OnceLock};

use agents_styles::nsh::{web_assets, web_script};
use axum::{
    Router,
    body::Body,
    extract::Request as AxumRequest,
    http::{StatusCode, header},
    response::{Html, IntoResponse, Response},
    routing::get,
};
use tower::ServiceExt;

use crate::modules::{
    gateway::controller::{GatewayHttpState, gateway_lazy_router},
    mcp::controller::{McpHttpState, protected_mcp_router},
    nodes::controller::{NodeHttpState, traefik_lazy_router},
};

/// Mounts the NSH Web UI shell (`/`, `/app`), static CSS/JS assets, and manifest.
pub fn webui_router() -> Router {
    Router::new()
        .route("/", get(serve_index))
        .route("/app", get(serve_index))
        .route("/assets/app.css", get(serve_css))
        .route("/assets/app.js", get(serve_js))
        .route("/manifest.webmanifest", get(serve_manifest))
        .route("/favicon.ico", get(serve_favicon))
}

/// Mounts full-duplex WebSocket stream, lazy MCP gateway, and self-hosted Scalar UI routes.
pub fn mount_native_stream_and_mcp_routes(
    router: Router,
    gw_cell: Arc<OnceLock<GatewayHttpState>>,
    mcp_cell: Arc<OnceLock<McpHttpState>>,
    nodes_cell: Arc<OnceLock<NodeHttpState>>,
    doc_cell: Arc<OnceLock<String>>,
) -> Router {
    let scalar_cfg = serde_json::json!({
        "url": "/openapi.json",
        "theme": "purple",
        "hideModels": false,
        "showSidebar": true
    });
    let scalar_cfg_slash = scalar_cfg.clone();
    let doc_cell_clone = Arc::clone(&doc_cell);

    router
        .merge(webui_router())
        .merge(gateway_lazy_router(gw_cell))
        .merge(traefik_lazy_router(nodes_cell))
        .merge(scalar_api_reference::axum::router("/docs", &scalar_cfg))
        .route(
            "/openapi.json",
            axum::routing::get(move || {
                let cell = Arc::clone(&doc_cell_clone);
                async move {
                    let doc = cell.get().cloned().unwrap_or_default();
                    (
                        [(axum::http::header::CONTENT_TYPE, "application/json")],
                        doc,
                    )
                }
            }),
        )
        .route(
            "/docs/",
            axum::routing::get(move || async move {
                scalar_api_reference::axum::scalar_response(
                    &scalar_cfg_slash,
                    Some("/docs/scalar.js"),
                )
            }),
        )
        .route(
            "/mcp",
            axum::routing::post(move |req: AxumRequest<Body>| {
                let mcp_cell = Arc::clone(&mcp_cell);
                async move { dispatch_lazy_mcp(req, mcp_cell).await }
            }),
        )
}

async fn dispatch_lazy_mcp(req: AxumRequest<Body>, cell: Arc<OnceLock<McpHttpState>>) -> Response {
    let Some(state) = cell.get() else {
        return (StatusCode::SERVICE_UNAVAILABLE, "mcp initializing").into_response();
    };
    let sub_router = protected_mcp_router(state.clone());
    match sub_router.oneshot(req).await {
        Ok(resp) => resp,
        Err(_) => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    }
}

async fn serve_index() -> Html<&'static str> {
    Html(web_assets::index_html())
}

async fn serve_css() -> Response {
    (
        StatusCode::OK,
        [(header::CONTENT_TYPE, "text/css; charset=utf-8")],
        web_assets::app_css(),
    )
        .into_response()
}

async fn serve_js() -> Response {
    (
        StatusCode::OK,
        [
            (
                header::CONTENT_TYPE,
                "application/javascript; charset=utf-8",
            ),
            (header::CACHE_CONTROL, "no-store"),
        ],
        web_script::app_js(),
    )
        .into_response()
}

async fn serve_manifest() -> Response {
    (
        StatusCode::OK,
        [(
            header::CONTENT_TYPE,
            "application/manifest+json; charset=utf-8",
        )],
        web_assets::web_manifest_json(),
    )
        .into_response()
}

async fn serve_favicon() -> Response {
    const SVG: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 32 32"><rect width="32" height="32" rx="6" fill="#b91c1c"/><text x="16" y="22" font-size="18" font-family="sans-serif" font-weight="bold" text-anchor="middle" fill="#fafaf9">A</text></svg>"##;
    (
        StatusCode::OK,
        [(header::CONTENT_TYPE, "image/svg+xml; charset=utf-8")],
        SVG,
    )
        .into_response()
}
