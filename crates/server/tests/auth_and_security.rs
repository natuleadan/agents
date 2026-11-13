#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use agents_server::{
    ServerConfig,
    modules::audit::services::AuditService,
    modules::auth::{
        repositories::AuthRepository,
        services::{AuthService, sha256_hex},
    },
};
use axum::http::{Method, StatusCode};
use serde_json::json;

use common::{build_test_harness, build_test_harness_with_config, register_and_login, send_json};

#[tokio::test]
async fn register_login_refresh_and_logout_lifecycle() {
    let harness = build_test_harness(ServerConfig::for_tests()).await;

    // Invalid email and short password fail garde validation with 422.
    let (bad_status, _) = send_json(
        &harness.router,
        Method::POST,
        "/api/v1/auth/register",
        None,
        Some(json!({ "email": "not-an-email", "password": "short" })),
    )
    .await;
    assert_eq!(bad_status, StatusCode::UNPROCESSABLE_ENTITY);

    let (reg_status, reg_body) = send_json(
        &harness.router,
        Method::POST,
        "/api/v1/auth/register",
        None,
        Some(json!({ "email": "alice@example.com", "password": "correct-horse-battery-123" })),
    )
    .await;
    assert_eq!(reg_status, StatusCode::OK);
    let alice_id = reg_body["userId"].as_str().unwrap().to_owned();

    // Duplicate registration returns 409 Conflict.
    let (dup_status, _) = send_json(
        &harness.router,
        Method::POST,
        "/api/v1/auth/register",
        None,
        Some(json!({ "email": "ALICE@example.com", "password": "correct-horse-battery-123" })),
    )
    .await;
    assert_eq!(dup_status, StatusCode::CONFLICT);

    // Wrong password returns 401 Unauthorized.
    let (wrong_pw_status, _) = send_json(
        &harness.router,
        Method::POST,
        "/api/v1/auth/login",
        None,
        Some(json!({ "email": "alice@example.com", "password": "wrong-password" })),
    )
    .await;
    assert_eq!(wrong_pw_status, StatusCode::UNAUTHORIZED);

    // Valid login issues JWT + opaque refresh token.
    let (login_status, tokens) = send_json(
        &harness.router,
        Method::POST,
        "/api/v1/auth/login",
        None,
        Some(json!({ "email": "alice@example.com", "password": "correct-horse-battery-123" })),
    )
    .await;
    assert_eq!(login_status, StatusCode::OK);
    let access = tokens["accessToken"].as_str().unwrap().to_owned();
    let refresh_1 = tokens["refreshToken"].as_str().unwrap().to_owned();

    // Unauthenticated `/me` returns 401.
    let (unauth_status, _) =
        send_json(&harness.router, Method::GET, "/api/v1/auth/me", None, None).await;
    assert_eq!(unauth_status, StatusCode::UNAUTHORIZED);

    // Authenticated `/me` returns current actor.
    let (me_status, me_body) = send_json(
        &harness.router,
        Method::GET,
        "/api/v1/auth/me",
        Some(&access),
        None,
    )
    .await;
    assert_eq!(me_status, StatusCode::OK);
    assert_eq!(me_body["userId"], alice_id);

    // Refresh rotates token; reusing old token is rejected.
    let (ref_status, rotated) = send_json(
        &harness.router,
        Method::POST,
        "/api/v1/auth/refresh",
        None,
        Some(json!({ "refreshToken": refresh_1 })),
    )
    .await;
    assert_eq!(ref_status, StatusCode::OK);
    let refresh_2 = rotated["refreshToken"].as_str().unwrap().to_owned();
    assert_ne!(refresh_1, refresh_2);

    let (reuse_status, _) = send_json(
        &harness.router,
        Method::POST,
        "/api/v1/auth/refresh",
        None,
        Some(json!({ "refreshToken": refresh_1 })),
    )
    .await;
    assert_eq!(reuse_status, StatusCode::UNAUTHORIZED);

    // Logout revokes refresh_2.
    let (logout_status, _) = send_json(
        &harness.router,
        Method::POST,
        "/api/v1/auth/logout",
        None,
        Some(json!({ "refreshToken": refresh_2 })),
    )
    .await;
    assert!(logout_status.is_success());

    let (post_logout_status, _) = send_json(
        &harness.router,
        Method::POST,
        "/api/v1/auth/refresh",
        None,
        Some(json!({ "refreshToken": refresh_2 })),
    )
    .await;
    assert_eq!(post_logout_status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn api_keys_store_sha256_digest_enforce_scopes_and_revoke_cleanly() {
    let harness = build_test_harness(ServerConfig::for_tests()).await;
    let (_, jwt) = register_and_login(
        &harness.router,
        "keys@example.com",
        "super-secure-password-1",
    )
    .await;

    // Mint a read-only API key.
    let (create_status, created) = send_json(
        &harness.router,
        Method::POST,
        "/api/v1/auth/api-keys",
        Some(&jwt),
        Some(json!({ "label": "ci-reader", "scopes": ["read"] })),
    )
    .await;
    assert_eq!(create_status, StatusCode::OK);
    let plaintext = created["plaintextKey"].as_str().unwrap().to_owned();
    let key_id = created["metadata"]["keyId"].as_str().unwrap().to_owned();
    assert!(plaintext.starts_with("ag_ak_"));

    // Plaintext secret is never stored; SHA-256 digest lookup resolves the record.
    let digest = sha256_hex(&plaintext);
    let stored = harness
        .auth_repo
        .find_api_key_by_hash(&digest)
        .await
        .unwrap()
        .expect("API key digest must exist");
    assert_ne!(stored.key_hash, plaintext);
    assert_eq!(stored.key_hash, digest);

    // Read endpoint succeeds; write endpoint is denied with 403 Forbidden.
    let (read_status, _) = send_json(
        &harness.router,
        Method::GET,
        "/api/v1/sessions",
        Some(&plaintext),
        None,
    )
    .await;
    assert_eq!(read_status, StatusCode::OK);

    let (write_status, _) = send_json(
        &harness.router,
        Method::POST,
        "/api/v1/sessions",
        Some(&plaintext),
        Some(json!({ "title": "unauthorized write" })),
    )
    .await;
    assert_eq!(write_status, StatusCode::FORBIDDEN);

    // Revoking via JWT immediately invalidates the API key (401).
    let (rev_status, _) = send_json(
        &harness.router,
        Method::DELETE,
        &format!("/api/v1/auth/api-keys/{key_id}"),
        Some(&jwt),
        None,
    )
    .await;
    assert_eq!(rev_status, StatusCode::OK);

    let (post_rev_status, _) = send_json(
        &harness.router,
        Method::GET,
        "/api/v1/sessions",
        Some(&plaintext),
        None,
    )
    .await;
    assert_eq!(post_rev_status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn login_rate_limit_and_admin_audit_hash_chain() {
    let mut cfg = ServerConfig::for_tests();
    cfg.rate_limit_max_requests = 2;
    cfg.rate_limit_window_secs = 60;
    cfg.admin_email = Some("admin@example.com".to_owned());
    cfg.admin_initial_password = Some("admin-secret-password-99".to_owned());

    let harness = build_test_harness_with_config(cfg).await;
    let (_, user_jwt) = register_and_login(
        &harness.router,
        "member@example.com",
        "member-password-1234",
    )
    .await;

    // Non-admin cannot access audit endpoints.
    let (non_admin_audit, _) = send_json(
        &harness.router,
        Method::GET,
        "/api/v1/audit/verify",
        Some(&user_jwt),
        None,
    )
    .await;
    assert_eq!(non_admin_audit, StatusCode::FORBIDDEN);

    // Log in as seeded admin and verify tamper-evident hash chain.
    let (admin_login_status, admin_tokens) = send_json(
        &harness.router,
        Method::POST,
        "/api/v1/auth/login",
        None,
        Some(json!({ "email": "admin@example.com", "password": "admin-secret-password-99" })),
    )
    .await;
    assert_eq!(admin_login_status, StatusCode::OK);
    let admin_jwt = admin_tokens["accessToken"].as_str().unwrap();

    let (verify_status, verify_body) = send_json(
        &harness.router,
        Method::GET,
        "/api/v1/audit/verify",
        Some(admin_jwt),
        None,
    )
    .await;
    assert_eq!(verify_status, StatusCode::OK);
    assert_eq!(verify_body["valid"], true);
    assert!(verify_body["checkedEntries"].as_u64().unwrap() >= 2);

    // Exhaust per-email login bucket -> 429 Too Many Requests.
    let _ = send_json(
        &harness.router,
        Method::POST,
        "/api/v1/auth/login",
        None,
        Some(json!({ "email": "member@example.com", "password": "bad-1" })),
    )
    .await;
    let (limited_status, _) = send_json(
        &harness.router,
        Method::POST,
        "/api/v1/auth/login",
        None,
        Some(json!({ "email": "member@example.com", "password": "bad-2" })),
    )
    .await;
    assert_eq!(limited_status, StatusCode::TOO_MANY_REQUESTS);

    // Verify AuditService::prune_expired resolves cleanly through DI.
    let audit = harness
        .app
        .container()
        .resolve::<AuditService>()
        .await
        .unwrap();
    let report = audit.verify_chain().await.unwrap();
    assert!(report.valid);
    let pruned = audit.prune_expired().await.unwrap();
    assert_eq!(pruned, 0);

    // Verify AuthService::authenticate_token rejects tampered JWTs.
    let auth = harness
        .app
        .container()
        .resolve::<AuthService>()
        .await
        .unwrap();
    assert!(auth.authenticate_token("tampered.jwt.token").await.is_err());
}
