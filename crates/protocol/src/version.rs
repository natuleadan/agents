//! Protocol versioning, named codecs, and capability negotiation contracts.

use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// Compatibility generation floor independent of crate binary versions.
pub const API_GENERATION: u32 = 1;

/// Named wire codec identifier with forward-compatible `Unknown` fallback.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum CodecId {
    SnapshotV1,
    EventsV1,
    Unknown(String),
}

impl CodecId {
    pub fn as_str(&self) -> &str {
        match self {
            Self::SnapshotV1 => "snapshot.v1",
            Self::EventsV1 => "events.v1",
            Self::Unknown(raw) => raw.as_str(),
        }
    }
}

impl Serialize for CodecId {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for CodecId {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let raw = String::deserialize(deserializer)?;
        Ok(match raw.as_str() {
            "snapshot.v1" => Self::SnapshotV1,
            "events.v1" => Self::EventsV1,
            _ => Self::Unknown(raw),
        })
    }
}

/// Optional platform capability advertised during handshake.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Capability {
    Streaming,
    LiveSteering,
    Swarm,
    McpHost,
    Autocron,
    TraefikPreview,
    ThirdPartyProviders,
    Unknown(String),
}

impl Capability {
    pub fn as_str(&self) -> &str {
        match self {
            Self::Streaming => "streaming",
            Self::LiveSteering => "live_steering",
            Self::Swarm => "swarm",
            Self::McpHost => "mcp_host",
            Self::Autocron => "autocron",
            Self::TraefikPreview => "traefik_preview",
            Self::ThirdPartyProviders => "third_party_providers",
            Self::Unknown(raw) => raw.as_str(),
        }
    }
}

impl Serialize for Capability {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for Capability {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let raw = String::deserialize(deserializer)?;
        Ok(match raw.as_str() {
            "streaming" => Self::Streaming,
            "live_steering" => Self::LiveSteering,
            "swarm" => Self::Swarm,
            "mcp_host" => Self::McpHost,
            "autocron" => Self::Autocron,
            "traefik_preview" => Self::TraefikPreview,
            "third_party_providers" => Self::ThirdPartyProviders,
            _ => Self::Unknown(raw),
        })
    }
}

/// Client handshake sent when opening an internal RPC or WebSocket session.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HandshakeRequest {
    pub client_name: String,
    pub client_version: String,
    pub api_generation: u32,
    pub requested_codecs: Vec<CodecId>,
    pub requested_capabilities: Vec<Capability>,
}

/// Server handshake declaring active generation, codecs, and capabilities.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HandshakeResponse {
    pub server_version: String,
    pub api_generation: u32,
    pub min_supported_generation: u32,
    pub active_codecs: Vec<CodecId>,
    pub capabilities: Vec<Capability>,
}

impl HandshakeResponse {
    /// Checks whether a given client generation meets the server floor.
    pub fn is_compatible_with(&self, client_generation: u32) -> bool {
        client_generation >= self.min_supported_generation
            && client_generation <= self.api_generation
    }

    /// Checks whether an optional capability is enabled on this connection.
    pub fn supports(&self, capability: &Capability) -> bool {
        self.capabilities.contains(capability)
    }
}
