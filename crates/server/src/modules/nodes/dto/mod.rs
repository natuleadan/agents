//! Data transfer objects for VLAN node registration and dynamic preview syncing.

use agents_protocol::{IsolationKind, NodeStatus, RuntimeKind, TraefikPreviewRoute};
use garde::Validate;
use serde::{Deserialize, Serialize};

/// Heartbeat payload sent by `agents-boxer` daemons to auto-register on the cluster.
#[derive(Debug, Clone, Deserialize, Serialize, Validate)]
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
#[derive(Debug, Clone, Deserialize, Serialize, Validate)]
#[serde(rename_all = "camelCase")]
pub struct PreviewSyncDto {
    #[garde(skip)]
    pub route: TraefikPreviewRoute,
    #[garde(skip)]
    pub token_sha256: Option<String>,
    #[garde(skip)]
    pub revoked: bool,
}
