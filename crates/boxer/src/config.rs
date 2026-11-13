//! Boxer runtime configuration, CLI arguments, private-VLAN validation, and two-axis defaults.

use std::{
    collections::HashMap,
    net::{IpAddr, Ipv4Addr, SocketAddr},
    path::PathBuf,
    time::{SystemTime, UNIX_EPOCH},
};

use agents_protocol::{IsolationKind, RuntimeKind, SandboxProfile, SetupMode, ThirdPartyProvider};
use clap::Parser;

use crate::error::BoxerError;

/// Command-line flags and environment bindings for `agents-boxer`.
#[derive(Debug, Clone, Parser)]
#[command(
    name = "agents-boxer",
    about = "Agents isolated sandbox supervisor daemon"
)]
pub struct BoxerCli {
    #[arg(long, env = "BOXER_NODE_ID", default_value = "boxer-local-01")]
    pub node_id: String,

    #[arg(long, env = "BOXER_SETUP_MODE", default_value = "local")]
    pub setup_mode: String,

    #[arg(long, env = "BOXER_RUNTIME", default_value = "docker")]
    pub runtime: String,

    #[arg(long, env = "BOXER_ISOLATION", default_value = "openshell")]
    pub isolation: String,

    #[arg(long, env = "BOXER_ALLOW_HOST_NATIVE", default_value_t = false)]
    pub allow_host_native: bool,

    #[arg(long, env = "BOXER_OPENSHELL_GATEWAY_URL")]
    pub openshell_gateway_url: Option<String>,

    #[arg(
        long,
        env = "BOXER_SANDBOX_IMAGE",
        default_value = "ghcr.io/natuleadan/agents-sandbox:latest"
    )]
    pub sandbox_image: String,

    #[arg(
        long,
        env = "BOXER_WORKSPACE_ROOT",
        default_value = "/tmp/agents-boxes"
    )]
    pub workspace_root: PathBuf,

    #[arg(long, env = "BOXER_MAX_CONCURRENT_BOXES", default_value_t = 8)]
    pub max_concurrent_boxes: usize,

    #[arg(long, env = "NATS_URL", default_value = "nats://127.0.0.1:4222")]
    pub nats_url: String,

    #[arg(long, env = "NATS_TLS_ENABLED", default_value_t = false)]
    pub nats_tls_enabled: bool,

    #[arg(long, env = "NATS_NKEY_SEED")]
    pub nats_nkey_seed: Option<String>,

    #[arg(long, env = "NATS_JWT")]
    pub nats_jwt: Option<String>,

    #[arg(
        long,
        env = "SERVER_INTERNAL_URL",
        default_value = "http://127.0.0.1:8080"
    )]
    pub server_internal_url: String,

    #[arg(long, env = "BOXER_WORKER_TOKEN", default_value = "dev-worker-token")]
    pub worker_token: String,

    #[arg(long, env = "BOXER_PROXY_BIND_ADDR", default_value = "127.0.0.1:0")]
    pub proxy_bind_addr: SocketAddr,

    #[arg(long, env = "BOXER_PRIVATE_VLAN_ONLY", default_value_t = true)]
    pub private_vlan_only: bool,

    #[arg(long, env = "TRAEFIK_PREVIEW_ENABLED", default_value_t = true)]
    pub traefik_preview_enabled: bool,

    #[arg(
        long,
        env = "TRAEFIK_PREVIEW_DOMAIN",
        default_value = "preview.example.com"
    )]
    pub traefik_preview_domain: String,

    #[arg(
        long,
        env = "TRAEFIK_DYNAMIC_DIR",
        default_value = "/tmp/agents-traefik-dynamic"
    )]
    pub traefik_dynamic_dir: PathBuf,

    #[arg(long, env = "TRAEFIK_DEFAULT_TTL_SECS", default_value_t = 900)]
    pub traefik_default_ttl_secs: u64,
}

/// Validated runtime configuration consumed by Boxer subsystems.
#[derive(Debug, Clone)]
pub struct BoxerConfig {
    pub node_id: String,
    pub default_profile: SandboxProfile,
    pub sandbox_image: String,
    pub workspace_root: PathBuf,
    pub max_concurrent_boxes: usize,
    pub nats_url: String,
    pub nats_tls_enabled: bool,
    pub nats_nkey_seed: Option<String>,
    pub nats_jwt: Option<String>,
    pub server_internal_url: String,
    pub worker_token: String,
    pub lease_ttl_seconds: u64,
    pub lease_heartbeat_seconds: u64,
    pub proxy_bind_addr: SocketAddr,
    pub private_vlan_only: bool,
    pub proxy_allowed_hosts: Vec<String>,
    pub proxy_upstream_headers: HashMap<String, String>,
    pub traefik_preview_enabled: bool,
    pub traefik_preview_domain: String,
    pub traefik_dynamic_dir: PathBuf,
    pub traefik_default_ttl_secs: u64,
    pub strict_native_isolation: bool,
}

impl BoxerConfig {
    /// Builds and validates [`BoxerConfig`] from parsed CLI arguments and environment variables.
    ///
    /// # Errors
    ///
    /// Returns [`BoxerError::Config`] or [`BoxerError::SandboxProfile`] if validation fails.
    pub fn from_cli(cli: BoxerCli) -> Result<Self, BoxerError> {
        let mode = parse_setup_mode(&cli.setup_mode)?;
        let runtime = parse_runtime_kind(&cli.runtime)?;
        let isolation = parse_isolation_kind(&cli.isolation)?;

        let default_profile = SandboxProfile {
            mode,
            runtime,
            isolation,
            allow_host_native: cli.allow_host_native,
            gateway_url: cli.openshell_gateway_url,
        };

        let proxy_allowed_hosts = std::env::var("BOXER_PROXY_ALLOWED_HOSTS")
            .unwrap_or_default()
            .split(',')
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(ToOwned::to_owned)
            .collect();

        let strict_native_isolation = std::env::var("BOXER_STRICT_NATIVE_ISOLATION")
            .map(|v| matches!(v.as_str(), "1" | "true" | "TRUE" | "yes"))
            .unwrap_or(false);

        let cfg = Self {
            node_id: cli.node_id,
            default_profile,
            sandbox_image: cli.sandbox_image,
            workspace_root: cli.workspace_root,
            max_concurrent_boxes: cli.max_concurrent_boxes,
            nats_url: cli.nats_url,
            nats_tls_enabled: cli.nats_tls_enabled,
            nats_nkey_seed: cli.nats_nkey_seed,
            nats_jwt: cli.nats_jwt,
            server_internal_url: cli.server_internal_url,
            worker_token: cli.worker_token,
            lease_ttl_seconds: 120,
            lease_heartbeat_seconds: 20,
            proxy_bind_addr: cli.proxy_bind_addr,
            private_vlan_only: cli.private_vlan_only,
            proxy_allowed_hosts,
            proxy_upstream_headers: HashMap::new(),
            traefik_preview_enabled: cli.traefik_preview_enabled,
            traefik_preview_domain: cli.traefik_preview_domain,
            traefik_dynamic_dir: cli.traefik_dynamic_dir,
            traefik_default_ttl_secs: cli.traefik_default_ttl_secs,
            strict_native_isolation,
        };
        cfg.validate()?;
        Ok(cfg)
    }

    /// Validates fail-closed Boxer invariants before daemon startup.
    ///
    /// # Errors
    ///
    /// Returns [`BoxerError`] when node ID, profile, concurrency, or VLAN bind rules fail.
    pub fn validate(&self) -> Result<(), BoxerError> {
        if self.node_id.trim().len() < 2 {
            return Err(BoxerError::Config(
                "BOXER_NODE_ID must contain at least 2 characters".into(),
            ));
        }
        if self.max_concurrent_boxes == 0 {
            return Err(BoxerError::Config(
                "BOXER_MAX_CONCURRENT_BOXES must be at least 1".into(),
            ));
        }
        if self.lease_heartbeat_seconds == 0
            || self.lease_heartbeat_seconds >= self.lease_ttl_seconds
        {
            return Err(BoxerError::Config(
                "lease_heartbeat_seconds must be > 0 and < lease_ttl_seconds".into(),
            ));
        }
        self.default_profile.validate()?;

        if self.default_profile.isolation == IsolationKind::Openshell
            && self
                .default_profile
                .gateway_url
                .as_deref()
                .map(str::trim)
                .is_none_or(str::is_empty)
        {
            return Err(BoxerError::Config(
                "openshell isolation requires BOXER_OPENSHELL_GATEWAY_URL".into(),
            ));
        }

        if self.private_vlan_only && !is_private_or_loopback_ip(self.proxy_bind_addr.ip()) {
            return Err(BoxerError::Config(format!(
                "refusing public bind address `{}` when private_vlan_only=true",
                self.proxy_bind_addr
            )));
        }
        Ok(())
    }
}

/// Checks whether an IP belongs to RFC1918 private VLAN (`10.0.0.0/8`, etc.) or loopback.
#[must_use]
pub fn is_private_or_loopback_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => is_private_or_loopback_v4(v4),
        IpAddr::V6(v6) => v6.is_loopback() || v6.is_unique_local(),
    }
}

#[must_use]
pub fn is_vlan_10_or_loopback(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => v4.is_loopback() || v4.octets()[0] == 10,
        IpAddr::V6(v6) => v6.is_loopback(),
    }
}

fn is_private_or_loopback_v4(v4: Ipv4Addr) -> bool {
    v4.is_loopback() || v4.is_private() || v4.is_link_local()
}

fn parse_setup_mode(raw: &str) -> Result<SetupMode, BoxerError> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "local" => Ok(SetupMode::Local),
        "remote" => Ok(SetupMode::Remote),
        other => Err(BoxerError::Config(format!(
            "unsupported setup mode `{other}`"
        ))),
    }
}

fn parse_runtime_kind(raw: &str) -> Result<RuntimeKind, BoxerError> {
    let trimmed = raw.trim().to_ascii_lowercase();
    match trimmed.as_str() {
        "host" => Ok(RuntimeKind::Host),
        "docker" => Ok(RuntimeKind::Docker),
        "remote_vps" | "remote-vps" => Ok(RuntimeKind::RemoteVps),
        "upstash_box" => Ok(RuntimeKind::ThirdParty(ThirdPartyProvider::UpstashBox)),
        "daytona" => Ok(RuntimeKind::ThirdParty(ThirdPartyProvider::Daytona)),
        "deepinfra" => Ok(RuntimeKind::ThirdParty(ThirdPartyProvider::DeepInfra)),
        "instabox" => Ok(RuntimeKind::ThirdParty(ThirdPartyProvider::Instabox)),
        other => Ok(RuntimeKind::Unknown(other.to_owned())),
    }
}

fn parse_isolation_kind(raw: &str) -> Result<IsolationKind, BoxerError> {
    Ok(match raw.trim().to_ascii_lowercase().as_str() {
        "none" => IsolationKind::None,
        "native" => IsolationKind::Native,
        "openshell" => IsolationKind::Openshell,
        other => IsolationKind::Unknown(other.to_owned()),
    })
}

/// Monotonic-safe wall-clock timestamp helper in milliseconds.
#[must_use]
pub fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
        .unwrap_or(0)
}
