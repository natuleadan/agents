//! Deterministic protocol schema export emitted directly from compiled contracts.

use serde::Serialize;

use crate::rpc::{METHOD_CATALOG, RpcMethodSpec};
use crate::sandbox::{IsolationKind, RuntimeKind, SandboxProfile, SetupMode};
use crate::version::{API_GENERATION, Capability, CodecId};

/// Validated matrix entry describing a supported `Runtime x Isolation` configuration.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SandboxMatrixEntry {
    pub mode: SetupMode,
    pub runtime: &'static str,
    pub isolation: IsolationKind,
    pub label: String,
}

/// Complete machine-readable protocol specification generated from Rust definitions.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProtocolSchemaBundle {
    pub api_generation: u32,
    pub codecs: Vec<CodecId>,
    pub capabilities: Vec<Capability>,
    pub rpc_methods: &'static [RpcMethodSpec],
    pub valid_sandbox_profiles: Vec<SandboxMatrixEntry>,
}

/// Builds the canonical protocol schema bundle for OpenAPI/codegen consumers.
pub fn export_protocol_schema() -> ProtocolSchemaBundle {
    let candidates = [
        SandboxProfile {
            mode: SetupMode::Local,
            runtime: RuntimeKind::Docker,
            isolation: IsolationKind::Openshell,
            allow_host_native: false,
            gateway_url: None,
        },
        SandboxProfile {
            mode: SetupMode::Local,
            runtime: RuntimeKind::Docker,
            isolation: IsolationKind::Native,
            allow_host_native: false,
            gateway_url: None,
        },
        SandboxProfile {
            mode: SetupMode::Local,
            runtime: RuntimeKind::Docker,
            isolation: IsolationKind::None,
            allow_host_native: false,
            gateway_url: None,
        },
        SandboxProfile {
            mode: SetupMode::Local,
            runtime: RuntimeKind::Host,
            isolation: IsolationKind::Native,
            allow_host_native: true,
            gateway_url: None,
        },
        SandboxProfile {
            mode: SetupMode::Local,
            runtime: RuntimeKind::Host,
            isolation: IsolationKind::None,
            allow_host_native: false,
            gateway_url: None,
        },
        SandboxProfile {
            mode: SetupMode::Remote,
            runtime: RuntimeKind::RemoteVps,
            isolation: IsolationKind::Openshell,
            allow_host_native: false,
            gateway_url: Some("https://gateway.internal:8443".to_owned()),
        },
    ];

    let valid_sandbox_profiles = candidates
        .into_iter()
        .filter(|profile| profile.validate().is_ok())
        .map(|profile| {
            let label = profile.effective_label();
            let runtime = match profile.runtime {
                RuntimeKind::Host => "host",
                RuntimeKind::Docker => "docker",
                RuntimeKind::RemoteVps => "remote_vps",
                RuntimeKind::ThirdParty(_) => "third_party",
                RuntimeKind::Unknown(_) => "unknown",
            };
            SandboxMatrixEntry {
                mode: profile.mode,
                runtime,
                isolation: profile.isolation,
                label,
            }
        })
        .collect();

    ProtocolSchemaBundle {
        api_generation: API_GENERATION,
        codecs: vec![CodecId::SnapshotV1, CodecId::EventsV1],
        capabilities: vec![
            Capability::Streaming,
            Capability::LiveSteering,
            Capability::Swarm,
            Capability::McpHost,
            Capability::Autocron,
            Capability::TraefikPreview,
            Capability::ThirdPartyProviders,
        ],
        rpc_methods: METHOD_CATALOG,
        valid_sandbox_profiles,
    }
}
