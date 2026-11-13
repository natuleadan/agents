#![allow(clippy::unwrap_used, clippy::expect_used)]

use agents_client::core::{
    ActiveView, AppAction, AppState, AuthState, BackendMode, ClusterNodeEntry, UpdateInfo, dispatch,
};
use agents_client::styles::{SurfaceMetrics, TargetPlatform};
use agents_client::ui::render_app_shell;
use agents_server::{InfrastructureHandles, ServerConfig, build_application};
use axum::{
    body::{Body, to_bytes},
    http::Request,
};
use tower::ServiceExt;

#[test]
fn login_screen_error_states_cluster_table_and_trace_render_cleanly() {
    let mut state = AppState::default();
    dispatch(
        &mut state,
        AppAction::SetLoginEmail("ops@example.com".to_owned()),
    );
    dispatch(
        &mut state,
        AppAction::SetAuthState(AuthState::RateLimited, None),
    );

    let login_tree = render_app_shell(&state, SurfaceMetrics::web(1440.0, 900.0));
    let login_html = login_tree.render_html_fragment();
    assert!(login_html.contains("data-testid=\"login-screen\""));
    assert!(login_html.contains("Rate limit exceeded"));
    assert!(login_tree.collect_trace().duplicate_test_ids.is_empty());

    dispatch(
        &mut state,
        AppAction::SetLoginMode(BackendMode::ThirdParty {
            provider: agents_protocol::ThirdPartyProvider::Daytona,
            endpoint: "https://daytona.example.com".to_owned(),
        }),
    );
    dispatch(
        &mut state,
        AppAction::SetAuthState(AuthState::Authenticated, None),
    );
    dispatch(
        &mut state,
        AppAction::SetClusterNodes(vec![ClusterNodeEntry {
            node_id: "boxer-mia-01".to_owned(),
            vlan_ip: "10.0.1.24".to_owned(),
            runtime_label: "docker/openshell".to_owned(),
            active_boxes: 2,
            max_boxes: 12,
            status_label: "ready".to_owned(),
        }]),
    );
    dispatch(
        &mut state,
        AppAction::RecordTelemetrySample {
            latency_ms: 19.5,
            tokens: 4200.0,
        },
    );
    dispatch(&mut state, AppAction::SetView(ActiveView::UpdateModal));
    dispatch(
        &mut state,
        AppAction::SetUpdateInfo(UpdateInfo {
            current_version: "0.1.0".to_owned(),
            available_version: Some("0.2.0".to_owned()),
            notes: "Signed release bundle".to_owned(),
        }),
    );

    for metrics in [
        SurfaceMetrics::web(1440.0, 900.0),
        SurfaceMetrics::web(834.0, 1194.0),
        SurfaceMetrics::mobile(TargetPlatform::Android),
    ] {
        let tree = render_app_shell(&state, metrics);
        let trace = tree.collect_trace();
        assert!(
            trace.duplicate_test_ids.is_empty(),
            "duplicate testIDs found: {:?}",
            trace.duplicate_test_ids
        );
        let html = tree.render_html_fragment();
        assert!(html.contains("boxer-mia-01"));
        assert!(html.contains("10.0.1.24"));
        assert!(html.contains("telemetry-latency-chart"));
        assert!(html.contains("Signed release bundle"));
        assert!(!html.contains("data-testid=\"login-screen\""));
    }
}

#[tokio::test]
async fn server_serves_webui_assets_without_inline_scripts() {
    let mut config = ServerConfig::for_tests();
    config.rate_limit_max_requests = 1000;
    let handles = InfrastructureHandles::from_config(config).await.unwrap();
    let app = build_application(handles).await.unwrap();
    let router = app.platform().router().clone();

    let index_resp = router
        .clone()
        .oneshot(Request::builder().uri("/").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(index_resp.status(), axum::http::StatusCode::OK);
    let body = String::from_utf8(
        to_bytes(index_resp.into_body(), 1024 * 1024)
            .await
            .unwrap()
            .to_vec(),
    )
    .unwrap();
    assert!(body.contains("data-testid=\"login-screen\""));
    assert!(body.contains("src=\"/assets/app.js\""));
    assert!(!body.contains("<script>"));

    let css_resp = router
        .clone()
        .oneshot(
            Request::builder()
                .uri("/assets/app.css")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(css_resp.status(), axum::http::StatusCode::OK);

    let js_resp = router
        .oneshot(
            Request::builder()
                .uri("/assets/app.js")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(js_resp.status(), axum::http::StatusCode::OK);
}
