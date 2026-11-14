//! Third-party cloud sandbox drivers tests for Phase 11.

use std::path::PathBuf;

use agents_boxer::config::BoxerConfig;
use agents_boxer::sandbox::{
    DeepInfraSandboxRuntimeDriver, SandboxDriver, SandboxExecRequest, UpstashBoxRuntimeDriver,
    build_sandbox_driver,
};
use agents_protocol::{
    ExecOptions, IsolationKind, RuntimeKind, SandboxProfile, SetupMode, ThirdPartyProvider,
};
use tokio::sync::watch;

fn test_profile(provider: ThirdPartyProvider) -> SandboxProfile {
    SandboxProfile {
        mode: SetupMode::Remote,
        runtime: RuntimeKind::ThirdParty(provider),
        isolation: IsolationKind::None,
        allow_host_native: false,
        gateway_url: None,
    }
}

fn test_exec_request(profile: SandboxProfile) -> SandboxExecRequest {
    SandboxExecRequest {
        task_id: "test-task-123".to_owned(),
        session_id: "test-session-456".to_owned(),
        command: "echo test".to_owned(),
        profile,
        exec: ExecOptions::default(),
        proxy_url: None,
    }
}

#[tokio::test]
async fn upstash_box_driver_rejects_missing_token_and_handles_cancel() {
    // Missing token fails with SandboxProfile error
    let unauth_driver = UpstashBoxRuntimeDriver::new(None);
    let profile = test_profile(ThirdPartyProvider::UpstashBox);
    let req = test_exec_request(profile);

    let (_tx, rx) = watch::channel(false);
    let err = unauth_driver
        .execute(req.clone(), None, rx)
        .await
        .expect_err("should fail without token");
    assert!(err.to_string().contains("profile") || err.to_string().contains("mismatch"));

    // Pre-cancelled execution returns code 130
    let driver = UpstashBoxRuntimeDriver::new(Some("mock-token".into()));
    let (_tx_cancel, rx_cancel) = watch::channel(true);
    let output = driver
        .execute(req, None, rx_cancel)
        .await
        .expect("pre-cancelled execution");
    assert_eq!(output.exit_code, 130);
    assert!(output.cancelled);
}

#[tokio::test]
async fn deepinfra_sandbox_driver_rejects_missing_token_and_handles_cancel() {
    // Missing token fails with SandboxProfile error
    let unauth_driver = DeepInfraSandboxRuntimeDriver::new(None);
    let profile = test_profile(ThirdPartyProvider::DeepInfra);
    let req = test_exec_request(profile);

    let (_tx, rx) = watch::channel(false);
    let err = unauth_driver
        .execute(req.clone(), None, rx)
        .await
        .expect_err("should fail without token");
    assert!(err.to_string().contains("profile") || err.to_string().contains("mismatch"));

    // Pre-cancelled execution returns code 130
    let driver = DeepInfraSandboxRuntimeDriver::new(Some("mock-token".into()));
    let (_tx_cancel, rx_cancel) = watch::channel(true);
    let output = driver
        .execute(req, None, rx_cancel)
        .await
        .expect("pre-cancelled execution");
    assert_eq!(output.exit_code, 130);
    assert!(output.cancelled);
}

#[test]
fn build_sandbox_driver_provisions_third_party_adapters() {
    let config = BoxerConfig::from_cli(agents_boxer::config::BoxerCli {
        node_id: "test-node".into(),
        setup_mode: "local".into(),
        runtime: "docker".into(),
        isolation: "none".into(),
        allow_host_native: false,
        openshell_gateway_url: None,
        sandbox_image: "test-image".into(),
        workspace_root: PathBuf::from("/tmp/test"),
        max_concurrent_boxes: 4,
        nats_url: "nats://127.0.0.1:4222".into(),
        nats_tls_enabled: false,
        nats_nkey_seed: None,
        nats_jwt: None,
        server_internal_url: "http://127.0.0.1:8080".into(),
        worker_token: "test".into(),
        proxy_bind_addr: "127.0.0.1:0".parse().unwrap(),
        private_vlan_only: true,
        traefik_preview_enabled: false,
        traefik_preview_domain: "preview.test".into(),
        traefik_dynamic_dir: PathBuf::from("/tmp/traefik"),
        traefik_default_ttl_secs: 900,
        upstash_box_token: Some("box-tok".into()),
        deepinfra_token: Some("deep-tok".into()),
        http_port: 9091,
        cluster_secret: "dev-boxer-cluster-secret".into(),
    })
    .expect("config");

    let upstash_profile = test_profile(ThirdPartyProvider::UpstashBox);
    let upstash_driver = build_sandbox_driver(&config, &upstash_profile);
    assert!(upstash_driver.is_ok());

    let deepinfra_profile = test_profile(ThirdPartyProvider::DeepInfra);
    let deepinfra_driver = build_sandbox_driver(&config, &deepinfra_profile);
    assert!(deepinfra_driver.is_ok());

    let unsupported_profile = test_profile(ThirdPartyProvider::Unknown("unknown-cloud".into()));
    let unsupported_res = build_sandbox_driver(&config, &unsupported_profile);
    assert!(unsupported_res.is_err());
}
