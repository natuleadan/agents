//! Lean internal JSON-RPC contracts, method catalog, and cursor pagination.

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json::Value;

use crate::version::Capability;

/// Request correlation identifier supporting either integer or string IDs.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(untagged)]
pub enum RequestId {
    Num(u64),
    Str(String),
}

/// Optional distributed trace context attached to internal RPC frames.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TraceContext {
    pub trace_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub span_id: Option<String>,
}

/// Lean internal RPC request envelope (omits redundant protocol version strings).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RpcRequest {
    pub id: RequestId,
    pub method: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub params: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trace: Option<TraceContext>,
}

/// Typed RPC error codes including admission backpressure (`Overloaded`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RpcErrorCode {
    Overloaded,
    Unauthorized,
    Forbidden,
    NotFound,
    Conflict,
    InvalidParams,
    UnsupportedGeneration,
    Internal,
    Unknown(String),
}

impl RpcErrorCode {
    pub fn as_str(&self) -> &str {
        match self {
            Self::Overloaded => "overloaded",
            Self::Unauthorized => "unauthorized",
            Self::Forbidden => "forbidden",
            Self::NotFound => "not_found",
            Self::Conflict => "conflict",
            Self::InvalidParams => "invalid_params",
            Self::UnsupportedGeneration => "unsupported_generation",
            Self::Internal => "internal",
            Self::Unknown(raw) => raw.as_str(),
        }
    }
}

impl Serialize for RpcErrorCode {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for RpcErrorCode {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let raw = String::deserialize(deserializer)?;
        Ok(match raw.as_str() {
            "overloaded" => Self::Overloaded,
            "unauthorized" => Self::Unauthorized,
            "forbidden" => Self::Forbidden,
            "not_found" => Self::NotFound,
            "conflict" => Self::Conflict,
            "invalid_params" => Self::InvalidParams,
            "unsupported_generation" => Self::UnsupportedGeneration,
            "internal" => Self::Internal,
            _ => Self::Unknown(raw),
        })
    }
}

/// Structured RPC error payload.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RpcError {
    pub code: RpcErrorCode,
    pub message: String,
    pub retryable: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub details: Option<Value>,
}

/// Lean internal RPC response envelope.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RpcResponse {
    pub id: RequestId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<RpcError>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trace: Option<TraceContext>,
}

/// Ordered RPC notification emitted from server/boxer streams.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RpcNotification {
    pub method: String,
    pub seq: u64,
    pub emitted_at_ms: u64,
    pub params: Value,
}

/// Concurrency serialization domain for each RPC method.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ExecutionScope {
    Session,
    Global,
    Readonly,
}

/// Declarative RPC method registration metadata.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RpcMethodSpec {
    pub method: &'static str,
    pub scope: ExecutionScope,
    pub min_generation: u32,
    pub required_capability: Option<Capability>,
    pub summary: &'static str,
}

/// Canonical declarative table of internal RPC methods (`domain/action`).
pub const METHOD_CATALOG: &[RpcMethodSpec] = &[
    RpcMethodSpec {
        method: "system/handshake",
        scope: ExecutionScope::Readonly,
        min_generation: 1,
        required_capability: None,
        summary: "Negotiate API generation, codecs, and capabilities",
    },
    RpcMethodSpec {
        method: "session/create",
        scope: ExecutionScope::Global,
        min_generation: 1,
        required_capability: None,
        summary: "Create a new conversation session",
    },
    RpcMethodSpec {
        method: "session/list",
        scope: ExecutionScope::Readonly,
        min_generation: 1,
        required_capability: None,
        summary: "List sessions with cursor pagination and tri-state filters",
    },
    RpcMethodSpec {
        method: "task/submit",
        scope: ExecutionScope::Session,
        min_generation: 1,
        required_capability: None,
        summary: "Enqueue a fenced task for sandbox execution",
    },
    RpcMethodSpec {
        method: "task/steer",
        scope: ExecutionScope::Session,
        min_generation: 1,
        required_capability: Some(Capability::LiveSteering),
        summary: "Inject live steering instructions into an active agent loop",
    },
    RpcMethodSpec {
        method: "task/cancel",
        scope: ExecutionScope::Session,
        min_generation: 1,
        required_capability: None,
        summary: "Cancel a running task and trigger process-group teardown",
    },
    RpcMethodSpec {
        method: "swarm/dispatch",
        scope: ExecutionScope::Session,
        min_generation: 1,
        required_capability: Some(Capability::Swarm),
        summary: "Delegate bounded work to a swarm subagent",
    },
    RpcMethodSpec {
        method: "preview/register",
        scope: ExecutionScope::Session,
        min_generation: 1,
        required_capability: Some(Capability::TraefikPreview),
        summary: "Publish an ephemeral Traefik route over private VLAN",
    },
];

/// Cursor-based pagination parameters.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PageRequest {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cursor: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<u32>,
}

/// Cursor-paginated response supporting forward and backward navigation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CursorPage<T> {
    pub items: Vec<T>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_cursor: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub backwards_cursor: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub total_hint: Option<u64>,
}

/// Tri-state filter distinguishing omitted (`Any`), explicit `null` (`None`), and value (`Exact`).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum TriStateFilter<T> {
    #[default]
    Any,
    None,
    Exact(T),
}

impl<T> TriStateFilter<T> {
    /// Deserializes an explicit JSON field value (`null` -> `None`, `T` -> `Exact(T)`).
    pub fn from_nullable_value<'de, D>(deserializer: D) -> Result<Self, D::Error>
    where
        T: Deserialize<'de>,
        D: Deserializer<'de>,
    {
        match Option::<T>::deserialize(deserializer)? {
            Some(val) => Ok(Self::Exact(val)),
            Option::None => Ok(Self::None),
        }
    }
}
