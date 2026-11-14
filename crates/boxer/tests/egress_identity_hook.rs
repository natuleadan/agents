#![allow(clippy::expect_used, clippy::unwrap_used)]
//! Tests for Boxer's transparent outbound installation identity egress hook.

use std::{collections::HashMap, sync::Arc};

use agents_boxer::proxy::{EgressIdentityPolicy, OutboundCredentialProxy};
use agents_protocol::{
    HEADER_AGENT_IDENTITY_CODE, HEADER_AGENT_INSTALLATION_ID, HEADER_AGENT_ORGANIZATION,
    InstallationIdentityHeaders,
};
use axum::{Json, Router, http::HeaderMap, routing::get};
use serde_json::{Value, json};
use tokio::{net::TcpListener, sync::Mutex};

#[tokio::test]
async fn boxer_proxy_strips_spoofed_headers_and_injects_host_identity() {
    let seen = Arc::new(Mutex::new(HashMap::<String, String>::new()));
    let seen_clone = Arc::clone(&seen);

    let upstream = Router::new().route(
        "/echo",
        get(move |headers: HeaderMap| {
            let seen = Arc::clone(&seen_clone);
            async move {
                let mut map = seen.lock().await;
                for name in [
                    HEADER_AGENT_ORGANIZATION,
                    HEADER_AGENT_INSTALLATION_ID,
                    HEADER_AGENT_IDENTITY_CODE,
                ] {
                    if let Some(val) = headers.get(name).and_then(|v| v.to_str().ok()) {
                        map.insert(name.to_owned(), val.to_owned());
                    }
                }
                Json(json!({ "ok": true }))
            }
        }),
    );

    let upstream_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let upstream_addr = upstream_listener.local_addr().unwrap();
    tokio::spawn(async move {
        let _ = axum::serve(upstream_listener, upstream).await;
    });

    let policy = EgressIdentityPolicy {
        identity: Some(InstallationIdentityHeaders {
            organization: "HostOrg".to_owned(),
            installation_id: "host-box-01".to_owned(),
            identity_code: Some("host-signed-code".to_owned()),
        }),
        strict_egress: true,
    };

    let proxy = OutboundCredentialProxy::start_with_identity(
        "127.0.0.1:0".parse().unwrap(),
        true,
        vec!["127.0.0.1".to_owned()],
        vec![],
        policy,
    )
    .await
    .expect("start proxy");

    let client = reqwest::Client::new();
    let resp = client
        .post(format!("{}/forward", proxy.endpoint_url()))
        .json(&json!({
            "method": "GET",
            "url": format!("http://{upstream_addr}/echo"),
            "headers": {
                "x-agent-organization": "SpoofedEvilOrg",
                "x-agent-installation-id": "spoof-999",
                "x-agent-identity-code": "fake-code"
            }
        }))
        .send()
        .await
        .expect("proxy forward");
    assert_eq!(resp.status(), reqwest::StatusCode::OK);

    let map = seen.lock().await;
    assert_eq!(
        map.get(HEADER_AGENT_ORGANIZATION).map(String::as_str),
        Some("HostOrg")
    );
    assert_eq!(
        map.get(HEADER_AGENT_INSTALLATION_ID).map(String::as_str),
        Some("host-box-01")
    );
    assert_eq!(
        map.get(HEADER_AGENT_IDENTITY_CODE).map(String::as_str),
        Some("host-signed-code")
    );
}

#[tokio::test]
async fn boxer_proxy_fails_closed_when_strict_egress_and_identity_missing() {
    let policy = EgressIdentityPolicy {
        identity: None,
        strict_egress: true,
    };

    let proxy = OutboundCredentialProxy::start_with_identity(
        "127.0.0.1:0".parse().unwrap(),
        true,
        vec!["127.0.0.1".to_owned()],
        vec![],
        policy,
    )
    .await
    .expect("start proxy");

    let client = reqwest::Client::new();
    let resp = client
        .post(format!("{}/forward", proxy.endpoint_url()))
        .json(&json!({
            "method": "GET",
            "url": "http://127.0.0.1:8080/health"
        }))
        .send()
        .await
        .expect("proxy forward");
    assert_eq!(resp.status(), reqwest::StatusCode::FORBIDDEN);
    let body: Value = resp.json().await.unwrap();
    assert!(
        body["error"]
            .as_str()
            .unwrap()
            .contains("installation identity")
    );
}
