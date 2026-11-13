#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::net::Ipv4Addr;
use std::path::PathBuf;

use agents_installer::cli::{Cli, Commands, execute_cli};
use agents_installer::config::{ClusterMode, TopologySpec};
use agents_installer::doctor::{DoctorReport, redact_secrets};
use agents_installer::envgen::{
    GeneratedSecrets, render_env_file, render_redacted_preview, write_env_file_secure,
};
use agents_installer::error::InstallerError;
use agents_installer::planner::ExecutionPlan;
use agents_installer::runner::{RecordingExecutor, SystemExecutor, run_plan};
use agents_protocol::{IsolationKind, RuntimeKind};
use clap::Parser;

fn repo_scripts_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("deploy/scripts")
}

fn sample_monolithic_spec() -> TopologySpec {
    TopologySpec {
        mode: ClusterMode::Monolithic,
        public_domain: Some("agents.example.com".to_owned()),
        preview_domain: "preview.example.com".to_owned(),
        acme_email: Some("ops@example.com".to_owned()),
        vlan_cidr: "10.0.0.0/8".to_owned(),
        vlan_ip: None,
        nats_url: "nats://127.0.0.1:4222".to_owned(),
        runtime: RuntimeKind::Docker,
        isolation: IsolationKind::Openshell,
        max_boxes: 8,
        ssh_target: None,
        ssh_key_path: None,
        scripts_dir: repo_scripts_dir(),
        env_file_path: PathBuf::from("/tmp/agents-test.env"),
        dry_run: true,
    }
}

#[test]
fn cli_parsing_supports_all_subcommands_and_modes() {
    let install = Cli::try_parse_from([
        "agents-installer",
        "install",
        "--mode",
        "server",
        "--public-domain",
        "api.example.com",
        "--acme-email",
        "sec@example.com",
        "--vlan-ip",
        "10.10.0.2",
        "--dry-run",
    ])
    .unwrap();
    assert!(matches!(
        install.command,
        Commands::Install { dry_run: true, .. }
    ));

    let join = Cli::try_parse_from([
        "agents-installer",
        "join-vlan",
        "--vlan-ip",
        "10.10.0.9",
        "--nats-url",
        "nats://10.10.0.2:4222",
        "--dry-run",
    ])
    .unwrap();
    assert!(matches!(join.command, Commands::JoinVlan { .. }));

    let doc = Cli::try_parse_from(["agents-installer", "doctor", "--mode", "boxer"]).unwrap();
    assert!(matches!(doc.command, Commands::Doctor { .. }));

    let upg = Cli::try_parse_from(["agents-installer", "upgrade", "--dry-run"]).unwrap();
    assert!(matches!(
        upg.command,
        Commands::Upgrade { dry_run: true, .. }
    ));
}

#[test]
fn topology_validation_enforces_fail_closed_security_and_vlan_rules() {
    let mut no_iso = sample_monolithic_spec();
    no_iso.isolation = IsolationKind::None;
    assert!(matches!(
        no_iso.validate(),
        Err(InstallerError::Validation(_))
    ));

    // Distributed Boxer refuses public IP as VLAN address.
    let mut public_boxer = sample_monolithic_spec();
    public_boxer.mode = ClusterMode::Boxer;
    public_boxer.vlan_ip = Some(Ipv4Addr::new(8, 8, 8, 8));
    public_boxer.nats_url = "nats://10.0.0.2:4222".to_owned();
    assert!(matches!(
        public_boxer.validate(),
        Err(InstallerError::Validation(_))
    ));

    // Path traversal blocked.
    let mut traversal = sample_monolithic_spec();
    traversal.env_file_path = PathBuf::from("../outside.env");
    assert!(matches!(
        traversal.validate(),
        Err(InstallerError::Validation(_))
    ));

    // Missing script fails closed.
    let mut missing_scripts = sample_monolithic_spec();
    missing_scripts.scripts_dir = PathBuf::from("/tmp/does-not-exist-scripts");
    assert!(matches!(
        ExecutionPlan::build_install(&missing_scripts),
        Err(InstallerError::MissingScript(_))
    ));
}

#[tokio::test]
async fn execution_plan_differs_cleanly_between_monolithic_server_and_boxer() {
    let mono = sample_monolithic_spec();
    let mono_plan = ExecutionPlan::build_install(&mono).unwrap();
    let mono_names: Vec<&str> = mono_plan
        .steps
        .iter()
        .map(|s| s.script_name.as_str())
        .collect();
    assert_eq!(
        mono_names,
        vec![
            "00-preflight.sh",
            "10-hardening.sh",
            "20-deps-server.sh",
            "30-deps-boxer.sh",
            "40-systemd-units.sh",
            "90-doctor.sh",
        ]
    );

    let mut srv = sample_monolithic_spec();
    srv.mode = ClusterMode::Server;
    srv.vlan_ip = Some(Ipv4Addr::new(10, 20, 0, 5));
    let srv_plan = ExecutionPlan::build_install(&srv).unwrap();
    assert!(
        srv_plan
            .steps
            .iter()
            .any(|s| s.script_name == "20-deps-server.sh")
    );
    assert!(
        !srv_plan
            .steps
            .iter()
            .any(|s| s.script_name == "30-deps-boxer.sh")
    );

    let mut boxer = sample_monolithic_spec();
    boxer.mode = ClusterMode::Boxer;
    boxer.public_domain = None;
    boxer.acme_email = None;
    boxer.vlan_ip = Some(Ipv4Addr::new(10, 20, 0, 6));
    boxer.nats_url = "nats://10.20.0.5:4222".to_owned();
    let boxer_plan = ExecutionPlan::build_join_vlan(&boxer).unwrap();
    assert!(
        !boxer_plan
            .steps
            .iter()
            .any(|s| s.script_name == "20-deps-server.sh")
    );
    assert!(
        boxer_plan
            .steps
            .iter()
            .any(|s| s.script_name == "30-deps-boxer.sh")
    );

    let exec = RecordingExecutor::with_stdout("[PASS] ok\n[SKIP] container");
    run_plan(&boxer_plan, &exec).await.unwrap();
    assert_eq!(exec.recorded_steps().len(), 5);
}

#[tokio::test]
async fn secrets_generation_redaction_permissions_and_dry_run_never_leak() {
    let spec = sample_monolithic_spec();
    let secrets = GeneratedSecrets::generate();
    assert_eq!(secrets.jwt_secret.len(), 64);
    assert!(secrets.nats_nkey_seed.starts_with("SU"));

    let dbg = format!("{secrets:?}");
    assert!(!dbg.contains(&secrets.jwt_secret));
    assert!(!dbg.contains(&secrets.credential_master_key));
    assert!(!dbg.contains(&secrets.postgres_password));

    let preview = render_redacted_preview(&spec, &secrets);
    assert!(preview.contains("AGENTS_JWT_SECRET=[REDACTED]"));
    assert!(!preview.contains(&secrets.jwt_secret));

    let redacted_log = redact_secrets(&format!(
        "running DATABASE_URL=postgres://agents:{}@127.0.0.1:5432/agents AGENTS_JWT_SECRET={}",
        secrets.postgres_password, secrets.jwt_secret
    ));
    assert!(!redacted_log.contains(&secrets.postgres_password));
    assert!(!redacted_log.contains(&secrets.jwt_secret));

    // Write secure 0600 file and test DryRun CLI command.
    let dir = std::env::temp_dir().join(format!("agents_env_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let env_path = dir.join("agents.env");
    let raw = render_env_file(&spec, &secrets);
    write_env_file_secure(&env_path, &raw).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&env_path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
    }

    let cli = Cli::try_parse_from([
        "agents-installer",
        "dry-run",
        "--mode",
        "monolithic",
        "--public-domain",
        "agents.example.com",
        "--acme-email",
        "ops@example.com",
        "--scripts-dir",
        repo_scripts_dir().to_str().unwrap(),
    ])
    .unwrap();
    let output = execute_cli(cli).await.unwrap();
    assert!(output.contains("00-preflight.sh"));
    assert!(output.contains("AGENTS_JWT_SECRET=[REDACTED]"));

    let _ = std::fs::remove_dir_all(dir);
}

#[tokio::test]
async fn real_preflight_dry_run_scripts_and_doctor_smoke_execute_cleanly() {
    let dir = std::env::temp_dir().join(format!("agents_scripts_smoke_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();

    // Prepare secure 0600 env file for 90-doctor.sh.
    let mut spec = sample_monolithic_spec();
    let env_path = dir.join("agents.env");
    spec.env_file_path = env_path.clone();
    let secrets = GeneratedSecrets::generate();
    write_env_file_secure(&env_path, &render_env_file(&spec, &secrets)).unwrap();

    let executor = SystemExecutor;
    let plan = ExecutionPlan::build_install(&spec).unwrap();
    for step in &plan.steps {
        let mut adjusted = step.clone();
        if step.script_name == "00-preflight.sh" {
            adjusted.args.push("--allow-non-root".to_owned());
        }
        let out = tokio::process::Command::new("bash")
            .arg(&adjusted.script_path)
            .args(&adjusted.args)
            .env("AGENTS_MIN_DISK_MB", "1")
            .env("AGENTS_MIN_RAM_MB", "64")
            .output()
            .await
            .unwrap();
        let stdout = String::from_utf8_lossy(&out.stdout);
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert!(
            out.status.success(),
            "script {} failed:\nstdout={stdout}\nstderr={stderr}",
            step.script_name
        );
        if step.script_name == "90-doctor.sh" {
            let report = DoctorReport::parse(&stdout);
            assert!(report.is_healthy());
            assert!(report.passed >= 3);
        }
    }

    // Verify 90-doctor.sh fails closed if .env permissions become world-readable (0644).
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&env_path, std::fs::Permissions::from_mode(0o644)).unwrap();
        let doctor_plan = ExecutionPlan::build_doctor(&spec).unwrap();
        let err = run_plan(&doctor_plan, &executor).await;
        // On failure `SystemExecutor` propagates Execution error; also confirm DoctorReport detects fail.
        assert!(err.is_err());
    }

    let _ = std::fs::remove_dir_all(dir);
}
