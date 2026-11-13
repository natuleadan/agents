use std::{collections::HashMap, net::SocketAddr, path::PathBuf, time::Duration};

use agents_boxer::{
    config::{BoxerCli, BoxerConfig},
    error::BoxerError,
    proxy::{CredentialRule, OutboundCredentialProxy},
    sandbox::{
        DockerRuntimeDriver, HostRuntimeDriver, SandboxDriver, SandboxExecRequest,
        build_sanitized_env, resolve_confined_workdir, validate_runtime_isolation,
    },
};
use agents_protocol::{
    ExecOptions, IsolationKind, RuntimeKind, SandboxProfile, SetupMode, ThirdPartyProvider,
};
use axum::{Router, extract::Request as AxumRequest, routing::get};
use bytes::Bytes;
use clap::Parser;
use http::{Request, header};
use http_body_util::{BodyExt, Full};
use hyper_util::{
    client::legacy::{Client, connect::HttpConnector},
    rt::TokioExecutor,
};
use tokio::{net::TcpListener, sync::watch};

type TestResult = Result<(), Box<dyn std::error::Error>>;

fn unique_temp_dir(prefix: &str) -> Result<PathBuf, Box<dyn std::error::Error>> {
    let dir = std::env::temp_dir().join(format!("{prefix}-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir)?;
    Ok(dir)
}

#[test]
fn runtime_isolation_matrix_fails_closed() {
    let valid_cases = vec![
        (
            SetupMode::Local,
            RuntimeKind::Host,
            IsolationKind::None,
            false,
            None,
        ),
        (
            SetupMode::Local,
            RuntimeKind::Host,
            IsolationKind::Native,
            true,
            None,
        ),
        (
            SetupMode::Local,
            RuntimeKind::Docker,
            IsolationKind::None,
            false,
            None,
        ),
        (
            SetupMode::Local,
            RuntimeKind::Docker,
            IsolationKind::Native,
            false,
            None,
        ),
        (
            SetupMode::Local,
            RuntimeKind::Docker,
            IsolationKind::Openshell,
            false,
            Some("https://openshell.internal:8443".to_owned()),
        ),
        (
            SetupMode::Remote,
            RuntimeKind::RemoteVps,
            IsolationKind::Native,
            false,
            None,
        ),
        (
            SetupMode::Remote,
            RuntimeKind::RemoteVps,
            IsolationKind::Openshell,
            false,
            Some("https://openshell.vlan:8443".to_owned()),
        ),
        (
            SetupMode::Remote,
            RuntimeKind::ThirdParty(ThirdPartyProvider::UpstashBox),
            IsolationKind::Openshell,
            false,
            Some("https://box.example.com".to_owned()),
        ),
    ];

    for (mode, runtime, isolation, allow_host_native, gateway_url) in valid_cases {
        let profile = SandboxProfile {
            mode,
            runtime,
            isolation,
            allow_host_native,
            gateway_url,
        };
        assert!(
            validate_runtime_isolation(&profile, false).is_ok(),
            "expected valid profile: {profile:?}"
        );
    }

    let host_openshell = SandboxProfile {
        mode: SetupMode::Local,
        runtime: RuntimeKind::Host,
        isolation: IsolationKind::Openshell,
        allow_host_native: true,
        gateway_url: Some("https://gw.internal".to_owned()),
    };
    assert!(validate_runtime_isolation(&host_openshell, false).is_err());

    let host_native_unopted = SandboxProfile {
        mode: SetupMode::Local,
        runtime: RuntimeKind::Host,
        isolation: IsolationKind::Native,
        allow_host_native: false,
        gateway_url: None,
    };
    assert!(validate_runtime_isolation(&host_native_unopted, false).is_err());

    let docker_openshell_no_gw = SandboxProfile {
        mode: SetupMode::Local,
        runtime: RuntimeKind::Docker,
        isolation: IsolationKind::Openshell,
        allow_host_native: false,
        gateway_url: None,
    };
    assert!(validate_runtime_isolation(&docker_openshell_no_gw, false).is_err());

    let remote_no_isolation = SandboxProfile {
        mode: SetupMode::Remote,
        runtime: RuntimeKind::RemoteVps,
        isolation: IsolationKind::None,
        allow_host_native: false,
        gateway_url: None,
    };
    assert!(validate_runtime_isolation(&remote_no_isolation, false).is_err());
}

#[test]
fn boxer_config_rejects_public_proxy_bind_when_vlan_only() -> TestResult {
    let mut cli = BoxerCli::parse_from([
        "agents-boxer",
        "--node-id",
        "boxer-test-01",
        "--openshell-gateway-url",
        "https://10.0.0.5:8443",
    ]);
    cli.proxy_bind_addr = "0.0.0.0:9090".parse()?;
    let res = BoxerConfig::from_cli(cli);
    assert!(matches!(res, Err(BoxerError::Config(_))));
    Ok(())
}

#[test]
fn vfs_confinement_blocks_traversal_sensitive_paths_and_symlink_escapes() -> TestResult {
    let root = unique_temp_dir("boxer-vfs-root")?;
    let outside = unique_temp_dir("boxer-outside")?;
    std::fs::write(outside.join("secret.txt"), "secret")?;

    let ok_dir = resolve_confined_workdir(&root, "sess-1", Some("project/app"))?;
    assert!(ok_dir.starts_with(std::fs::canonicalize(&root)?));

    assert!(resolve_confined_workdir(&root, "sess-1", Some("../escape")).is_err());
    assert!(resolve_confined_workdir(&root, "sess-1", Some("/etc/passwd")).is_err());
    assert!(resolve_confined_workdir(&root, "sess-1", Some(".ssh/id_ed25519")).is_err());
    assert!(resolve_confined_workdir(&root, "sess-1", Some("sub/.env")).is_err());

    #[cfg(unix)]
    {
        let link_path = root.join("sess-1").join("escape_link");
        std::os::unix::fs::symlink(&outside, &link_path)?;
        let symlink_res = resolve_confined_workdir(&root, "sess-1", Some("escape_link"));
        assert!(
            symlink_res.is_err(),
            "symlink escaping session root must fail"
        );
    }

    let _ = std::fs::remove_dir_all(root);
    let _ = std::fs::remove_dir_all(outside);
    Ok(())
}

#[test]
fn sanitized_env_scrubs_secrets_from_allowlist_and_docker_spec_hardened() -> TestResult {
    let exec = ExecOptions {
        working_dir: Some("app".to_owned()),
        timeout_seconds: Some(30),
        max_memory_mb: Some(256),
        max_cpus: Some(1),
        env_allowlist: vec![
            "OPENAI_API_KEY".to_owned(),
            "AGENTS_JWT_SECRET".to_owned(),
            "CUSTOM_TOKEN".to_owned(),
            "HOME".to_owned(),
        ],
        network_egress: false,
    };

    let env = build_sanitized_env(&exec, Some("http://127.0.0.1:9999"));
    assert!(!env.contains_key("OPENAI_API_KEY"));
    assert!(!env.contains_key("AGENTS_JWT_SECRET"));
    assert!(!env.contains_key("CUSTOM_TOKEN"));
    assert!(
        !env.contains_key("HTTP_PROXY"),
        "egress=false omits proxy env"
    );

    let root = unique_temp_dir("boxer-docker-spec")?;
    let driver = DockerRuntimeDriver::new(root.clone(), "ghcr.io/natuleadan/agents-sandbox:v1");
    let spec = driver.build_command_spec(&SandboxExecRequest {
        task_id: "task-123".to_owned(),
        session_id: "sess-a".to_owned(),
        command: "echo hi".to_owned(),
        profile: SandboxProfile {
            mode: SetupMode::Local,
            runtime: RuntimeKind::Docker,
            isolation: IsolationKind::Openshell,
            allow_host_native: false,
            gateway_url: Some("https://10.0.0.9:8443".to_owned()),
        },
        exec,
        proxy_url: None,
    })?;

    assert!(spec.args.contains(&"--read-only".to_owned()));
    assert!(spec.args.contains(&"--cap-drop=ALL".to_owned()));
    assert!(
        spec.args
            .contains(&"--security-opt=no-new-privileges:true".to_owned())
    );
    assert!(spec.args.contains(&"--network=none".to_owned()));
    assert!(spec.args.contains(&"--memory=256m".to_owned()));
    assert!(
        spec.args
            .contains(&"agents.openshell.gateway=https://10.0.0.9:8443".to_owned())
    );

    let _ = std::fs::remove_dir_all(root);
    Ok(())
}

#[tokio::test]
async fn host_sandbox_hides_raw_secrets_and_terminates_process_group_on_cancel() -> TestResult {
    let root = unique_temp_dir("boxer-host-run")?;
    let driver = HostRuntimeDriver::new(root.clone(), false);

    let (_tx, rx) = watch::channel(false);
    let out = driver
        .execute(
            SandboxExecRequest {
                task_id: "task-env-check".to_owned(),
                session_id: "sess-env".to_owned(),
                command:
                    "printf 'KEY=%s|PATH_SET=%s' \"${OPENAI_API_KEY:-ABSENT}\" \"${PATH:+YES}\""
                        .to_owned(),
                profile: SandboxProfile {
                    mode: SetupMode::Local,
                    runtime: RuntimeKind::Host,
                    isolation: IsolationKind::None,
                    allow_host_native: false,
                    gateway_url: None,
                },
                exec: ExecOptions {
                    env_allowlist: vec![
                        "OPENAI_API_KEY".to_owned(),
                        "AGENTS_JWT_SECRET".to_owned(),
                    ],
                    timeout_seconds: Some(10),
                    ..Default::default()
                },
                proxy_url: None,
            },
            None,
            rx,
        )
        .await?;

    assert_eq!(out.exit_code, 0);
    assert!(out.stdout.contains("KEY=ABSENT|PATH_SET=YES"));

    // Cancellation mid-sleep triggers process-group termination promptly.
    let (cancel_tx, cancel_rx) = watch::channel(false);
    let root_cancel = root.clone();
    let driver_cancel = HostRuntimeDriver::new(root_cancel, false);
    let handle = tokio::spawn(async move {
        driver_cancel
            .execute(
                SandboxExecRequest {
                    task_id: "task-cancel".to_owned(),
                    session_id: "sess-cancel".to_owned(),
                    command: "sleep 30".to_owned(),
                    profile: SandboxProfile {
                        mode: SetupMode::Local,
                        runtime: RuntimeKind::Host,
                        isolation: IsolationKind::None,
                        allow_host_native: false,
                        gateway_url: None,
                    },
                    exec: ExecOptions {
                        timeout_seconds: Some(45),
                        ..Default::default()
                    },
                    proxy_url: None,
                },
                None,
                cancel_rx,
            )
            .await
    });

    tokio::time::sleep(Duration::from_millis(80)).await;
    let _ = cancel_tx.send(true);
    let cancel_out = tokio::time::timeout(Duration::from_secs(3), handle).await???;
    assert!(cancel_out.cancelled);

    let _ = std::fs::remove_dir_all(root);
    Ok(())
}

#[tokio::test]
async fn outbound_credential_proxy_injects_tokens_outside_sandbox_and_blocks_ssrf() -> TestResult {
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let upstream_addr = listener.local_addr()?;

    let upstream_app = Router::new().route(
        "/v1/models",
        get(|req: AxumRequest| async move {
            req.headers()
                .get(header::AUTHORIZATION)
                .and_then(|v| v.to_str().ok())
                .unwrap_or("missing")
                .to_owned()
        }),
    );
    let upstream_task = tokio::spawn(async move {
        let _ = axum::serve(listener, upstream_app).await;
    });

    let proxy_bind: SocketAddr = "127.0.0.1:0".parse()?;
    let proxy = OutboundCredentialProxy::start(
        proxy_bind,
        true,
        vec!["127.0.0.1".to_owned()],
        vec![CredentialRule {
            host: "127.0.0.1".to_owned(),
            header_name: "Authorization".to_owned(),
            header_value: "Bearer upstream-secret-token".to_owned(),
        }],
    )
    .await?;

    let mut connector = HttpConnector::new();
    connector.enforce_http(false);
    let client: Client<HttpConnector, Full<Bytes>> =
        Client::builder(TokioExecutor::new()).build(connector);

    // 1) Allowed host gets upstream secret injected by proxy (caller sent none).
    let ok_payload = serde_json::json!({
        "method": "GET",
        "url": format!("http://{}/v1/models", upstream_addr),
        "headers": HashMap::<String, String>::from([("Authorization".to_owned(), "spoofed".to_owned())]),
    });
    let req = Request::post(format!("{}/forward", proxy.endpoint_url()))
        .header(header::CONTENT_TYPE, "application/json")
        .body(Full::new(Bytes::from(serde_json::to_vec(&ok_payload)?)))?;
    let resp = client.request(req).await?;
    assert_eq!(resp.status().as_u16(), 200);
    let body = String::from_utf8(resp.into_body().collect().await?.to_bytes().to_vec())?;
    assert_eq!(body, "Bearer upstream-secret-token");

    // 2) Metadata IP SSRF blocked fail-closed.
    let ssrf_payload = serde_json::json!({
        "method": "GET",
        "url": "http://169.254.169.254/latest/meta-data/",
    });
    let ssrf_req = Request::post(format!("{}/forward", proxy.endpoint_url()))
        .header(header::CONTENT_TYPE, "application/json")
        .body(Full::new(Bytes::from(serde_json::to_vec(&ssrf_payload)?)))?;
    let ssrf_resp = client.request(ssrf_req).await?;
    assert_eq!(ssrf_resp.status().as_u16(), 403);

    // 3) Disallowed external host blocked fail-closed.
    let denied_payload = serde_json::json!({
        "method": "GET",
        "url": "http://evil.example.org/exfiltrate",
    });
    let denied_req = Request::post(format!("{}/forward", proxy.endpoint_url()))
        .header(header::CONTENT_TYPE, "application/json")
        .body(Full::new(Bytes::from(serde_json::to_vec(&denied_payload)?)))?;
    let denied_resp = client.request(denied_req).await?;
    assert_eq!(denied_resp.status().as_u16(), 403);

    upstream_task.abort();
    Ok(())
}
