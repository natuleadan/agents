//! Data transfer objects for VLAN node registration and dynamic preview syncing.

use agents_protocol::{IsolationKind, NodeStatus, RuntimeKind, TraefikPreviewRoute};
use garde::Validate;
use ironic::OpenApiSchema;
use serde::{Deserialize, Serialize};

/// Heartbeat payload sent by `agents-boxer` daemons to auto-register on the cluster.
#[derive(Debug, Clone, Deserialize, Serialize, Validate, OpenApiSchema)]
#[serde(rename_all = "camelCase")]
pub struct NodeHeartbeatDto {
    #[garde(length(min = 2, max = 128))]
    pub node_id: String,
    #[garde(length(min = 3, max = 64))]
    pub vlan_ip: String,
    #[garde(skip)]
    pub runtime: RuntimeKind,
    #[garde(skip)]
    pub isolation: IsolationKind,
    #[garde(range(min = 1, max = 1024))]
    pub max_concurrent_boxes: usize,
    #[garde(skip)]
    pub active_boxes: usize,
    #[garde(skip)]
    pub status: NodeStatus,
    #[garde(range(min = 5, max = 3600))]
    pub ttl_seconds: Option<u64>,
}

/// Distributed Traefik preview route upsert/revocation synchronized by a Boxer node.
#[derive(Debug, Clone, Deserialize, Serialize, Validate, OpenApiSchema)]
#[serde(rename_all = "camelCase")]
pub struct PreviewSyncDto {
    #[garde(skip)]
    pub route: TraefikPreviewRoute,
    #[garde(skip)]
    pub token_sha256: Option<String>,
    #[garde(skip)]
    pub revoked: bool,
}

/// Mutual discovery registration payload sent by a Boxer daemon with HMAC authentication.
#[derive(Debug, Clone, Deserialize, Serialize, Validate, OpenApiSchema)]
#[serde(rename_all = "camelCase")]
pub struct NodeRegistrationDto {
    #[garde(length(min = 2, max = 128))]
    pub node_id: String,
    #[garde(length(min = 3, max = 64))]
    pub node_ip: String,
    #[garde(range(min = 1, max = 65535))]
    pub port: u16,
    #[garde(skip)]
    pub runtime: RuntimeKind,
    #[garde(skip)]
    pub isolation: IsolationKind,
    #[garde(range(min = 1, max = 1024))]
    pub max_concurrent_boxes: usize,
    /// Timestamp in epoch milliseconds.
    #[garde(range(min = 1))]
    pub timestamp_ms: u64,
    /// HMAC-SHA256 signature in lowercase hex over `{node_id}:{node_ip}:{port}:{timestamp_ms}`.
    #[garde(length(min = 64, max = 64))]
    pub signature: String,
}

/// Acknowledgment response returned to Boxer node with dynamic lease credentials.
#[derive(Debug, Clone, Deserialize, Serialize, OpenApiSchema)]
#[serde(rename_all = "camelCase")]
pub struct NodeRegistrationResponse {
    pub node_id: String,
    pub session_token: String,
    pub expires_at: u64,
    pub status: String,
}
