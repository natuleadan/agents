use std::net::Ipv4Addr;
use std::path::PathBuf;

use agents_protocol::{IsolationKind, RuntimeKind};
use clap::{Args, Parser, Subcommand};

use crate::config::{ClusterMode, TopologySpec};
use crate::doctor::DoctorReport;
use crate::envgen::{
    GeneratedSecrets, render_env_file, render_redacted_preview, write_env_file_secure,
};
use crate::error::InstallerError;
use crate::planner::ExecutionPlan;
use crate::runner::{SystemExecutor, run_plan};

#[derive(Debug, Parser)]
#[command(
    name = "agents-installer",
    about = "Hardened VPS provisioning and diagnostic CLI for agents clusters"
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Debug, Clone, Args)]
pub struct TopologyArgs {
    #[arg(long, value_enum, default_value_t = ClusterMode::Monolithic)]
    pub mode: ClusterMode,
    #[arg(long)]
    pub public_domain: Option<String>,
    #[arg(long, default_value = "preview.example.com")]
    pub preview_domain: String,
    #[arg(long)]
    pub acme_email: Option<String>,
    #[arg(long, default_value = "10.0.0.0/8")]
    pub vlan_cidr: String,
    #[arg(long)]
    pub vlan_ip: Option<Ipv4Addr>,
    #[arg(long, default_value = "nats://127.0.0.1:4222")]
    pub nats_url: String,
    #[arg(long, default_value = "docker")]
    pub runtime: String,
    #[arg(long, default_value = "openshell")]
    pub isolation: String,
    #[arg(long, default_value_t = 8)]
    pub max_boxes: u16,
    #[arg(long)]
    pub ssh_target: Option<String>,
    #[arg(long)]
    pub ssh_key_path: Option<PathBuf>,
    #[arg(long, default_value = "deploy/scripts")]
    pub scripts_dir: PathBuf,
    #[arg(long, default_value = "/etc/agents/agents.env")]
    pub env_file_path: PathBuf,
}

impl TopologyArgs {
    /// Converts CLI arguments into a [`TopologySpec`].
    ///
    /// # Errors
    ///
    /// Propagates validation errors during [`TopologySpec::validate`].
    pub fn into_spec(self, dry_run: bool) -> Result<TopologySpec, InstallerError> {
        let runtime = parse_runtime(&self.runtime);
        let isolation = parse_isolation(&self.isolation);
        let spec = TopologySpec {
            mode: self.mode,
            public_domain: self.public_domain,
            preview_domain: self.preview_domain,
            acme_email: self.acme_email,
            vlan_cidr: self.vlan_cidr,
            vlan_ip: self.vlan_ip,
            nats_url: self.nats_url,
            runtime,
            isolation,
            max_boxes: self.max_boxes,
            ssh_target: self.ssh_target,
            ssh_key_path: self.ssh_key_path,
            scripts_dir: self.scripts_dir,
            env_file_path: self.env_file_path,
            dry_run,
        };
        Ok(spec)
    }
}

#[derive(Debug, Clone, Subcommand)]
pub enum Commands {
    /// Provision and harden a Monolithic or Distributed VPS node using deploy/scripts/*.sh.
    Install {
        #[command(flatten)]
        args: TopologyArgs,
        #[arg(long)]
        dry_run: bool,
    },
    /// Join an additional private VLAN Boxer node to an existing orchestrator cluster.
    JoinVlan {
        #[command(flatten)]
        args: TopologyArgs,
        #[arg(long)]
        dry_run: bool,
    },
    /// Run non-destructive health and security invariants audit via 90-doctor.sh.
    Doctor {
        #[command(flatten)]
        args: TopologyArgs,
    },
    /// Execute preflight, unit refresh, and doctor checks for an existing node upgrade.
    Upgrade {
        #[command(flatten)]
        args: TopologyArgs,
        #[arg(long)]
        dry_run: bool,
    },
    /// Validate topology, preview redacted .env, and print ordered execution plan cleanly.
    DryRun {
        #[command(flatten)]
        args: TopologyArgs,
    },
}

fn parse_runtime(raw: &str) -> RuntimeKind {
    match raw {
        "host" => RuntimeKind::Host,
        "docker" => RuntimeKind::Docker,
        "remote-vps" => RuntimeKind::RemoteVps,
        other => RuntimeKind::Unknown(other.to_owned()),
    }
}

fn parse_isolation(raw: &str) -> IsolationKind {
    match raw {
        "none" => IsolationKind::None,
        "native" => IsolationKind::Native,
        "openshell" => IsolationKind::Openshell,
        other => IsolationKind::Unknown(other.to_owned()),
    }
}

/// Runs the CLI command cleanly without leaking generated secrets to stdout or logs.
///
/// # Errors
///
/// Returns [`InstallerError`] if topology validation, env file persistence, or script execution fails.
pub async fn execute_cli(cli: Cli) -> Result<String, InstallerError> {
    let executor = SystemExecutor;
    match cli.command {
        Commands::DryRun { args } => {
            let spec = args.into_spec(true)?;
            let plan = ExecutionPlan::build_install(&spec)?;
            let secrets = GeneratedSecrets::generate();
            let env_preview = render_redacted_preview(&spec, &secrets);
            let plan_json = serde_json::to_string_pretty(&plan)
                .map_err(|e| InstallerError::Validation(e.to_string()))?;
            Ok(format!(
                "{plan_json}\n\n# Redacted .env preview\n{env_preview}"
            ))
        }
        Commands::Install { args, dry_run } => {
            let spec = args.into_spec(dry_run)?;
            let plan = ExecutionPlan::build_install(&spec)?;
            if !dry_run && spec.ssh_target.is_none() {
                let secrets = GeneratedSecrets::generate();
                let rendered = render_env_file(&spec, &secrets);
                write_env_file_secure(&spec.env_file_path, &rendered)?;
            }
            let outputs = run_plan(&plan, &executor).await?;
            Ok(format!(
                "Completed {} script step(s) in {:?} mode (dry_run={dry_run})",
                outputs.len(),
                spec.mode
            ))
        }
        Commands::JoinVlan { mut args, dry_run } => {
            args.mode = ClusterMode::Boxer;
            let spec = args.into_spec(dry_run)?;
            let plan = ExecutionPlan::build_join_vlan(&spec)?;
            let outputs = run_plan(&plan, &executor).await?;
            Ok(format!(
                "Joined Boxer VLAN node via {} step(s) (dry_run={dry_run})",
                outputs.len()
            ))
        }
        Commands::Doctor { args } => {
            let spec = args.into_spec(true)?;
            let plan = ExecutionPlan::build_doctor(&spec)?;
            let outputs = run_plan(&plan, &executor).await?;
            let combined = outputs
                .into_iter()
                .map(|o| o.stdout)
                .collect::<Vec<_>>()
                .join("\n");
            let report = DoctorReport::parse(&combined);
            serde_json::to_string_pretty(&report)
                .map_err(|e| InstallerError::Validation(e.to_string()))
        }
        Commands::Upgrade { args, dry_run } => {
            let spec = args.into_spec(dry_run)?;
            let plan = ExecutionPlan::build_upgrade(&spec)?;
            let outputs = run_plan(&plan, &executor).await?;
            Ok(format!(
                "Upgrade verification completed ({} step(s), dry_run={dry_run})",
                outputs.len()
            ))
        }
    }
}
