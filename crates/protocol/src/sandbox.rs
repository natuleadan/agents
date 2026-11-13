//! Two-axis sandbox execution profile (`Runtime x Isolation`) and options.

use std::fmt;

use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// Target operating mode chosen by the user during setup.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SetupMode {
    Local,
    Remote,
}

/// Supported external sandbox-as-a-service providers in third-party client mode.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum ThirdPartyProvider {
    UpstashBox,
    Daytona,
    DeepInfra,
    Instabox,
    Unknown(String),
}

impl ThirdPartyProvider {
    pub fn as_str(&self) -> &str {
        match self {
            Self::UpstashBox => "upstash_box",
            Self::Daytona => "daytona",
            Self::DeepInfra => "deepinfra",
            Self::Instabox => "instabox",
            Self::Unknown(raw) => raw.as_str(),
        }
    }
}

impl Serialize for ThirdPartyProvider {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for ThirdPartyProvider {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let raw = String::deserialize(deserializer)?;
        Ok(match raw.as_str() {
            "upstash_box" => Self::UpstashBox,
            "daytona" => Self::Daytona,
            "deepinfra" => Self::DeepInfra,
            "instabox" => Self::Instabox,
            _ => Self::Unknown(raw),
        })
    }
}

/// Execution substrate running the sandbox workload.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "provider", rename_all = "camelCase")]
pub enum RuntimeKind {
    Host,
    Docker,
    RemoteVps,
    ThirdParty(ThirdPartyProvider),
    Unknown(String),
}

impl RuntimeKind {
    pub fn label(&self) -> &str {
        match self {
            Self::Host => "host",
            Self::Docker => "docker",
            Self::RemoteVps => "remote_vps",
            Self::ThirdParty(p) => p.as_str(),
            Self::Unknown(raw) => raw.as_str(),
        }
    }
}

/// Policy and confinement layer enforced around the runtime.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IsolationKind {
    None,
    Native,
    Openshell,
    Unknown(String),
}

impl IsolationKind {
    pub fn as_str(&self) -> &str {
        match self {
            Self::None => "none",
            Self::Native => "native",
            Self::Openshell => "openshell",
            Self::Unknown(raw) => raw.as_str(),
        }
    }
}

impl Serialize for IsolationKind {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for IsolationKind {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let raw = String::deserialize(deserializer)?;
        Ok(match raw.as_str() {
            "none" => Self::None,
            "native" => Self::Native,
            "openshell" => Self::Openshell,
            _ => Self::Unknown(raw),
        })
    }
}

/// Errors emitted when an invalid or unsafe `Runtime x Isolation` pair is configured.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SandboxProfileError {
    OpenshellRequiresContainerOrRemote,
    HostNativeRequiresExplicitOptIn,
    ModeRuntimeMismatch,
    UnknownProfileComponent,
}

impl fmt::Display for SandboxProfileError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::OpenshellRequiresContainerOrRemote => {
                write!(f, "openshell isolation requires docker or remote runtime")
            }
            Self::HostNativeRequiresExplicitOptIn => {
                write!(f, "host + native isolation requires explicit local opt-in")
            }
            Self::ModeRuntimeMismatch => {
                write!(f, "setup mode does not match selected runtime substrate")
            }
            Self::UnknownProfileComponent => {
                write!(
                    f,
                    "unrecognized runtime or isolation component cannot validate"
                )
            }
        }
    }
}

impl std::error::Error for SandboxProfileError {}

/// Explicit, fail-closed sandbox execution profile.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SandboxProfile {
    pub mode: SetupMode,
    pub runtime: RuntimeKind,
    pub isolation: IsolationKind,
    #[serde(default)]
    pub allow_host_native: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gateway_url: Option<String>,
}

impl SandboxProfile {
    /// Validates the two-axis matrix without any silent fallback.
    pub fn validate(&self) -> Result<(), SandboxProfileError> {
        if matches!(self.runtime, RuntimeKind::Unknown(_))
            || matches!(self.isolation, IsolationKind::Unknown(_))
        {
            return Err(SandboxProfileError::UnknownProfileComponent);
        }

        match (self.mode, &self.runtime) {
            (SetupMode::Local, RuntimeKind::Host | RuntimeKind::Docker) => {}
            (SetupMode::Remote, RuntimeKind::RemoteVps | RuntimeKind::ThirdParty(_)) => {}
            _ => return Err(SandboxProfileError::ModeRuntimeMismatch),
        }

        match (&self.runtime, &self.isolation) {
            (RuntimeKind::Host, IsolationKind::Openshell) => {
                Err(SandboxProfileError::OpenshellRequiresContainerOrRemote)
            }
            (RuntimeKind::Host, IsolationKind::Native) if !self.allow_host_native => {
                Err(SandboxProfileError::HostNativeRequiresExplicitOptIn)
            }
            _ => Ok(()),
        }
    }

    /// Formats the user-visible effective profile badge (`local · docker + openshell`).
    pub fn effective_label(&self) -> String {
        let mode_label = match self.mode {
            SetupMode::Local => "local",
            SetupMode::Remote => "remote",
        };
        format!(
            "{mode_label} · {} + {}",
            self.runtime.label(),
            self.isolation.as_str()
        )
    }
}

/// Resource and environment constraints passed to Boxer for command execution.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct ExecOptions {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub working_dir: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout_seconds: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_memory_mb: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_cpus: Option<u32>,
    #[serde(default)]
    pub env_allowlist: Vec<String>,
    #[serde(default)]
    pub network_egress: bool,
}
