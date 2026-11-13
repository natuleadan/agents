//! Static Web UI asset routes backed cleanly by [`agents_styles`] token & bundle generators.

use agents_styles::nsh::{web_assets, web_script};
use axum::{
    Router,
    http::{StatusCode, header},
    response::{Html, IntoResponse, Response},
    routing::get,
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
