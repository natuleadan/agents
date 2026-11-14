#![allow(clippy::expect_used, clippy::unwrap_used)]
//! Integration and anti-regression tests for singleton installation identity,
//! indefinite interaction ledger, inbound webhook logging, and A2A v1 postal gateway.

use std::sync::Arc;

use agents_protocol::{
    HEADER_AGENT_IDENTITY_CODE, HEADER_AGENT_INSTALLATION_ID, HEADER_AGENT_ORGANIZATION,
};
use agents_server::{
    app::{InfrastructureHandles, build_application},
    infra::config::ServerConfig,
};
use axum::{
    Json, Router,
    body::Body,
    http::{HeaderMap, Request, StatusCode},
    routing::post,
};
use http_body_util::BodyExt;
use serde_json::{Value, json};
use tokio::{net::TcpListener, sync::Mutex};
use tower::ServiceExt;

async fn setup_test_router() -> (Router, String) {
    let mut config = ServerConfig::for_tests();
    config.admin_email = Some("a2a-admin@example.com".to_owned());
    config.admin_initial_password = Some("AdminStrongPass!123456".to_owned());
    let handles = InfrastructureHandles::in_memory(config);
    let app = build_application(handles)
        .await
        .expect("build application for a2a/identity test");
    let router = app.platform().router().clone();

    let login_req = Request::builder()
        .method("POST")
        .uri("/api/v1/auth/login")
        .header("content-type", "application/json")
        .body(Body::from(
            json!({
                "email": "a2a-admin@example.com",
                "password": "AdminStrongPass!123456"
            })
            .to_string(),
        ))
        .expect("login req");
    let login_resp = router.clone().oneshot(login_req).await.expect("login");
    assert_eq!(login_resp.status(), StatusCode::OK);
    let body_bytes = login_resp
        .into_body()
        .collect()
        .await
        .expect("body")
        .to_bytes();
    let body: Value = serde_json::from_slice(&body_bytes).expect("json");
    let access_token = body["accessToken"]
        .as_str()
        .expect("accessToken")
        .to_owned();

    (router, access_token)
}

#[tokio::test]
async fn a2a_agent_card_exposes_organization_and_identity_extension() {
    let (router, api_key) = setup_test_router().await;

    let put_req = Request::builder()
        .method("PUT")
        .uri("/api/v1/identity/installation")
        .header("authorization", format!("Bearer {api_key}"))
        .header("content-type", "application/json")
        .body(Body::from(
            json!({
                "organizationName": "Acme Corp",
                "installationId": "acme-inst-001",
                "identityCode": "signed-cert-abc123",
                "strictInboundWebhooks": false,
                "strictOutboundEgress": true
            })
            .to_string(),
        ))
        .expect("put identity");
    let put_resp = router.clone().oneshot(put_req).await.expect("put resp");
    assert_eq!(put_resp.status(), StatusCode::OK);
    let put_bytes = put_resp.into_body().collect().await.unwrap().to_bytes();
    let put_json: Value = serde_json::from_slice(&put_bytes).unwrap();
    assert_eq!(put_json["hasIdentityCode"], true);
    assert!(
        put_json.get("identityCode").is_none(),
        "raw identity code must never be serialized"
    );

    let card_req = Request::builder()
        .method("GET")
        .uri("/.well-known/agent-card.json")
        .header("host", "agents.example.com")
        .body(Body::empty())
        .expect("card req");
    let card_resp = router.clone().oneshot(card_req).await.expect("card resp");
    assert_eq!(card_resp.status(), StatusCode::OK);
    let card_bytes = card_resp.into_body().collect().await.unwrap().to_bytes();
    let card: Value = serde_json::from_slice(&card_bytes).unwrap();
    assert_eq!(card["provider"]["organization"], "Acme Corp");
    assert_eq!(
        card["supportedInterfaces"][0]["url"],
        "https://agents.example.com/a2a"
    );
    assert_eq!(
        card["capabilities"]["extensions"][0]["uri"],
        "urn:agents:installation-identity:v1"
    );
}

#[tokio::test]
async fn a2a_postal_gateway_processes_message_send_and_records_indefinite_ledger() {
    let (router, api_key) = setup_test_router().await;

    let rpc_req = Request::builder()
        .method("POST")
        .uri("/a2a")
        .header("content-type", "application/json")
        .header(HEADER_AGENT_ORGANIZATION, "PartnerOrg")
        .header(HEADER_AGENT_INSTALLATION_ID, "partner-99")
        .header(HEADER_AGENT_IDENTITY_CODE, "sig-xyz-789")
        .body(Body::from(
            json!({
                "jsonrpc": "2.0",
                "id": "req-1",
                "method": "message/send",
                "params": {
                    "message": {
                        "messageId": "msg-1",
                        "role": "ROLE_USER",
                        "parts": [{ "text": "Hello orchestrator postal gateway" }]
                    }
                }
            })
            .to_string(),
        ))
        .expect("rpc req");
    let rpc_resp = router.clone().oneshot(rpc_req).await.expect("rpc resp");
    assert_eq!(rpc_resp.status(), StatusCode::OK);
    let rpc_bytes = rpc_resp.into_body().collect().await.unwrap().to_bytes();
    let rpc_json: Value = serde_json::from_slice(&rpc_bytes).unwrap();
    let task_id = rpc_json["result"]["id"]
        .as_str()
        .expect("task id")
        .to_owned();
    assert_eq!(
        rpc_json["result"]["status"]["state"],
        "TASK_STATE_COMPLETED"
    );

    let get_req = Request::builder()
        .method("POST")
        .uri("/a2a")
        .header("content-type", "application/json")
        .body(Body::from(
            json!({
                "jsonrpc": "2.0",
                "id": "req-2",
                "method": "tasks/get",
                "params": { "id": task_id }
            })
            .to_string(),
        ))
        .expect("tasks/get");
    let get_resp = router.clone().oneshot(get_req).await.expect("get resp");
    assert_eq!(get_resp.status(), StatusCode::OK);

    let ledger_req = Request::builder()
        .method("GET")
        .uri("/api/v1/identity/ledger?protocol=a2a")
        .header("authorization", format!("Bearer {api_key}"))
        .body(Body::empty())
        .expect("ledger req");
    let ledger_resp = router
        .clone()
        .oneshot(ledger_req)
        .await
        .expect("ledger resp");
    assert_eq!(ledger_resp.status(), StatusCode::OK);
    let ledger_bytes = ledger_resp.into_body().collect().await.unwrap().to_bytes();
    let ledger: Value = serde_json::from_slice(&ledger_bytes).unwrap();
    assert!(ledger["count"].as_u64().unwrap() >= 2);
    let verified = ledger["entries"]
        .as_array()
        .unwrap()
        .iter()
        .find(|e| e["remoteOrg"] == "PartnerOrg")
        .expect("PartnerOrg entry");
    assert_eq!(verified["verificationStatus"], "verified");
    assert_eq!(verified["remoteInstallationId"], "partner-99");
}

#[tokio::test]
async fn inbound_webhook_without_identity_headers_is_allowed_and_logged_as_missing() {
    let (router, api_key) = setup_test_router().await;

    let wh_req = Request::builder()
        .method("POST")
        .uri("/webhooks/external-hook-demo")
        .header("content-type", "application/json")
        .body(Body::from(
            json!({ "payload": { "event": "ping" } }).to_string(),
        ))
        .expect("wh req");
    let wh_resp = router.clone().oneshot(wh_req).await.expect("wh resp");
    // Slug doesn't exist -> 404 from workflow lookup, NOT 401/403 from missing identity headers
    assert_eq!(wh_resp.status(), StatusCode::NOT_FOUND);

    let ledger_req = Request::builder()
        .method("GET")
        .uri("/api/v1/identity/ledger?protocol=webhook")
        .header("authorization", format!("Bearer {api_key}"))
        .body(Body::empty())
        .expect("ledger req");
    let ledger_resp = router
        .clone()
        .oneshot(ledger_req)
        .await
        .expect("ledger resp");
    assert_eq!(ledger_resp.status(), StatusCode::OK);
    let ledger_bytes = ledger_resp.into_body().collect().await.unwrap().to_bytes();
    let ledger: Value = serde_json::from_slice(&ledger_bytes).unwrap();
    assert_eq!(ledger["count"], 1);
    assert_eq!(ledger["entries"][0]["verificationStatus"], "missing");
    assert_eq!(
        ledger["entries"][0]["endpointPath"],
        "/webhooks/external-hook-demo"
    );
}

#[tokio::test]
async fn outbound_a2a_injects_installation_headers_and_fails_closed_when_unconfigured() {
    let (router, api_key) = setup_test_router().await;

    // 1. Fail-closed before identity is configured
    let fail_req = Request::builder()
        .method("POST")
        .uri("/api/v1/a2a/send")
        .header("authorization", format!("Bearer {api_key}"))
        .header("content-type", "application/json")
        .body(Body::from(
            json!({
                "targetUrl": "http://127.0.0.1:9/a2a",
                "text": "Should fail closed"
            })
            .to_string(),
        ))
        .expect("fail req");
    let fail_resp = router.clone().oneshot(fail_req).await.expect("fail resp");
    assert_eq!(fail_resp.status(), StatusCode::FORBIDDEN);

    // 2. Configure singleton identity and start local mock A2A receiver
    let put_req = Request::builder()
        .method("PUT")
        .uri("/api/v1/identity/installation")
        .header("authorization", format!("Bearer {api_key}"))
        .header("content-type", "application/json")
        .body(Body::from(
            json!({
                "organizationName": "SenderOrg",
                "installationId": "sender-inst-77",
                "identityCode": "cert-sender-77",
                "strictInboundWebhooks": false,
                "strictOutboundEgress": true
            })
            .to_string(),
        ))
        .expect("put identity");
    assert_eq!(
        router.clone().oneshot(put_req).await.unwrap().status(),
        StatusCode::OK
    );

    let captured = Arc::new(Mutex::new(Vec::<(String, String, String)>::new()));
    let cap_clone = Arc::clone(&captured);
    let mock_app = Router::new().route(
        "/a2a",
        post(move |headers: HeaderMap, Json(body): Json<Value>| {
            let cap = Arc::clone(&cap_clone);
            async move {
                let org = headers
                    .get(HEADER_AGENT_ORGANIZATION)
                    .and_then(|v| v.to_str().ok())
                    .unwrap_or("")
                    .to_owned();
                let id = headers
                    .get(HEADER_AGENT_INSTALLATION_ID)
                    .and_then(|v| v.to_str().ok())
                    .unwrap_or("")
                    .to_owned();
                let code = headers
                    .get(HEADER_AGENT_IDENTITY_CODE)
                    .and_then(|v| v.to_str().ok())
                    .unwrap_or("")
                    .to_owned();
                cap.lock().await.push((org, id, code));
                Json(json!({ "jsonrpc": "2.0", "id": body["id"], "result": { "ok": true } }))
            }
        }),
    );
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        let _ = axum::serve(listener, mock_app).await;
    });

    let send_req = Request::builder()
        .method("POST")
        .uri("/api/v1/a2a/send")
        .header("authorization", format!("Bearer {api_key}"))
        .header("content-type", "application/json")
        .body(Body::from(
            json!({
                "targetUrl": format!("http://{addr}/a2a"),
                "text": "Hello remote agent"
            })
            .to_string(),
        ))
        .expect("send req");
    let send_resp = router.clone().oneshot(send_req).await.expect("send resp");
    assert_eq!(send_resp.status(), StatusCode::OK);

    let received = captured.lock().await;
    assert_eq!(received.len(), 1);
    assert_eq!(received[0].0, "SenderOrg");
    assert_eq!(received[0].1, "sender-inst-77");
    assert_eq!(received[0].2, "cert-sender-77");
}
