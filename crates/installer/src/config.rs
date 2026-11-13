use std::net::{IpAddr, Ipv4Addr};
use std::path::{Path, PathBuf};

use agents_protocol::{IsolationKind, RuntimeKind};
use clap::ValueEnum;
use serde::{Deserialize, Serialize};

use crate::error::InstallerError;

/// Cluster provisioning topology mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ClusterMode {
    /// Mode A: single hardened VPS running Server + Boxer + datastores + Traefik.
    Monolithic,
    /// Mode B control plane: public Server VPS exposing HTTPS/WSS + VLAN NATS/Valkey.
    Server,
    /// Mode B worker node: private VLAN Boxer VPS executing sandboxes with zero public ports.
    Boxer,
}

impl ClusterMode {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Monolithic => "monolithic",
            Self::Server => "server",
            Self::Boxer => "boxer",
        }
    }

    #[must_use]
    pub const fn includes_server(self) -> bool {
        matches!(self, Self::Monolithic | Self::Server)
    }

    #[must_use]
    pub const fn includes_boxer(self) -> bool {
        matches!(self, Self::Monolithic | Self::Boxer)
    }
}

/// Validated provisioning topology without silent fallbacks between Mode A and Mode B.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TopologySpec {
    pub mode: ClusterMode,
    pub public_domain: Option<String>,
    pub preview_domain: String,
    pub acme_email: Option<String>,
    pub vlan_cidr: String,
    pub vlan_ip: Option<Ipv4Addr>,
    pub nats_url: String,
    pub runtime: RuntimeKind,
    pub isolation: IsolationKind,
    pub max_boxes: u16,
    pub ssh_target: Option<String>,
    pub ssh_key_path: Option<PathBuf>,
    pub scripts_dir: PathBuf,
    pub env_file_path: PathBuf,
    pub dry_run: bool,
}

impl TopologySpec {
    /// Validates Mode A / Mode B network and isolation invariants fail-closed.
    ///
    /// # Errors
    ///
    /// Returns [`InstallerError::Validation`] if domains, VLAN subnets, SSH targets,
    /// or Boxer isolation combinations violate security invariants.
    pub fn validate(&self) -> Result<(), InstallerError> {
        validate_cidr_v4(&self.vlan_cidr)?;
        validate_safe_token("preview_domain", &self.preview_domain)?;
        validate_boxer_isolation(&self.runtime, &self.isolation)?;

        if self.mode.includes_server() {
            let domain = self.public_domain.as_deref().ok_or_else(|| {
                InstallerError::Validation(
                    "server and monolithic modes require --public-domain".to_owned(),
                )
            })?;
            validate_safe_token("public_domain", domain)?;

            let email = self.acme_email.as_deref().ok_or_else(|| {
                InstallerError::Validation(
                    "server and monolithic modes require --acme-email for TLS".to_owned(),
                )
            })?;
            validate_email(email)?;
        }

        match self.mode {
            ClusterMode::Monolithic => {
                if !self.nats_url.starts_with("nats://127.0.0.1:")
                    && !self.nats_url.starts_with("nats://localhost:")
                {
                    return Err(InstallerError::Validation(
                        "monolithic mode requires loopback NATS URL (nats://127.0.0.1:4222)"
                            .to_owned(),
                    ));
                }
            }
            ClusterMode::Server | ClusterMode::Boxer => {
                let vip = self.vlan_ip.ok_or_else(|| {
                    InstallerError::Validation(format!(
                        "distributed {:?} mode requires a private --vlan-ip inside {}",
                        self.mode, self.vlan_cidr
                    ))
                })?;
                if !is_private_rfc1918_or_cgnat(vip) || !ip_in_cidr(vip, &self.vlan_cidr)? {
                    return Err(InstallerError::Validation(format!(
                        "vlan_ip {vip} must be a private IPv4 address inside {}",
                        self.vlan_cidr
                    )));
                }
                if self.mode == ClusterMode::Boxer && !self.nats_url.starts_with("nats://") {
                    return Err(InstallerError::Validation(
                        "distributed boxer mode requires explicit VLAN --nats-url (nats://...)"
                            .to_owned(),
                    ));
                }
            }
        }

        if let Some(target) = &self.ssh_target {
            validate_ssh_target(target)?;
        }
        if let Some(key) = &self.ssh_key_path {
            validate_no_parent_traversal("ssh_key_path", key)?;
        }
        validate_no_parent_traversal("env_file_path", &self.env_file_path)?;

        Ok(())
    }
}

fn validate_boxer_isolation(
    runtime: &RuntimeKind,
    isolation: &IsolationKind,
) -> Result<(), InstallerError> {
    if *isolation == IsolationKind::None {
        return Err(InstallerError::Validation(
            "fail-closed: installer refuses IsolationKind::None in production provisioning"
                .to_owned(),
        ));
    }
    if matches!(runtime, RuntimeKind::ThirdParty(_)) {
        return Err(InstallerError::Validation(
            "VPS installer does not provision ThirdParty runtimes".to_owned(),
        ));
    }
    let setup_mode = if *runtime == RuntimeKind::RemoteVps {
        agents_protocol::SetupMode::Remote
    } else {
        agents_protocol::SetupMode::Local
    };
    agents_protocol::SandboxProfile {
        mode: setup_mode,
        runtime: runtime.clone(),
        isolation: isolation.clone(),
        allow_host_native: true,
        gateway_url: None,
    }
    .validate()
    .map_err(|e| InstallerError::Validation(format!("unsafe runtime/isolation matrix: {e}")))?;
    Ok(())
}

fn validate_safe_token(label: &str, value: &str) -> Result<(), InstallerError> {
    if value.is_empty()
        || !value
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_'))
    {
        return Err(InstallerError::Validation(format!(
            "{label} contains unsafe characters"
        )));
    }
    Ok(())
}

fn validate_email(email: &str) -> Result<(), InstallerError> {
    let valid = email.contains('@')
        && email.contains('.')
        && !email.chars().any(char::is_whitespace)
        && email
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '@' | '.' | '-' | '_' | '+'));
    if !valid {
        return Err(InstallerError::Validation(
            "acme_email must be a valid email address".to_owned(),
        ));
    }
    Ok(())
}

fn validate_ssh_target(target: &str) -> Result<(), InstallerError> {
    let valid = !target.is_empty()
        && !target.starts_with('-')
        && target
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '@' | '.' | '-' | '_'));
    if !valid {
        return Err(InstallerError::Validation(
            "ssh_target must match user@host without whitespace or option prefixes".to_owned(),
        ));
    }
    Ok(())
}

fn validate_no_parent_traversal(label: &str, path: &Path) -> Result<(), InstallerError> {
    if path
        .components()
        .any(|c| matches!(c, std::path::Component::ParentDir))
    {
        return Err(InstallerError::Validation(format!(
            "{label} must not contain '..' path traversal"
        )));
    }
    Ok(())
}

fn parse_cidr_v4(cidr: &str) -> Result<(Ipv4Addr, u8), InstallerError> {
    let (ip_str, prefix_str) = cidr.split_once('/').ok_or_else(|| {
        InstallerError::Validation(format!("invalid IPv4 CIDR '{cidr}' (missing '/')"))
    })?;
    let ip: IpAddr = ip_str
        .parse()
        .map_err(|_| InstallerError::Validation(format!("invalid CIDR IP '{ip_str}'")))?;
    let IpAddr::V4(v4) = ip else {
        return Err(InstallerError::Validation(
            "VLAN CIDR must be IPv4".to_owned(),
        ));
    };
    let prefix: u8 = prefix_str
        .parse()
        .map_err(|_| InstallerError::Validation(format!("invalid CIDR prefix '{prefix_str}'")))?;
    if prefix > 32 {
        return Err(InstallerError::Validation(format!(
            "CIDR prefix out of range: /{prefix}"
        )));
    }
    Ok((v4, prefix))
}

/// Validates private IPv4 CIDR formatting.
///
/// # Errors
///
/// Returns [`InstallerError::Validation`] if `cidr` is not valid private IPv4 CIDR notation.
pub fn validate_cidr_v4(cidr: &str) -> Result<(), InstallerError> {
    let (v4, prefix) = parse_cidr_v4(cidr)?;
    if prefix < 8 || !is_private_rfc1918_or_cgnat(v4) {
        return Err(InstallerError::Validation(format!(
            "VLAN CIDR {cidr} must be a private RFC1918/CGNAT range (/8 or narrower)"
        )));
    }
    Ok(())
}

#[must_use]
pub fn is_private_rfc1918_or_cgnat(ip: Ipv4Addr) -> bool {
    let [a, b, _, _] = ip.octets();
    a == 10
        || (a == 172 && (16..=31).contains(&b))
        || (a == 192 && b == 168)
        || (a == 100 && (64..=127).contains(&b))
}

fn ip_in_cidr(ip: Ipv4Addr, cidr: &str) -> Result<bool, InstallerError> {
    let (net, prefix) = parse_cidr_v4(cidr)?;
    let mask = if prefix == 0 {
        0u32
    } else {
        u32::MAX << (32 - u32::from(prefix))
    };
    Ok((u32::from(ip) & mask) == (u32::from(net) & mask))
}
