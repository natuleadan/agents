//! Worker node discovery, private-VLAN heartbeat, and dynamic preview sync contracts.

use serde::{Deserialize, Serialize};

use crate::{
    preview::TraefikPreviewRoute,
    sandbox::{IsolationKind, RuntimeKind},
};

/// Operational lifecycle state advertised by a Boxer worker over the control plane.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NodeStatus {
    #[default]
    Online,
    Draining,
    Offline,
}

/// Auto-registration heartbeat announced by any private-VLAN `agents-boxer` instance.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BoxerNodeAnnouncement {
    pub node_id: String,
    pub vlan_ip: String,
    pub runtime: RuntimeKind,
    pub isolation: IsolationKind,
    pub max_concurrent_boxes: usize,
    pub active_boxes: usize,
    pub status: NodeStatus,
    pub last_heartbeat_ms: u64,
    pub ttl_seconds: u64,
}

/// Dynamic Traefik route update synchronized from a Boxer node to the control plane.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PreviewRouteSync {
    pub route: TraefikPreviewRoute,
    pub token_sha256: Option<String>,
    pub revoked: bool,
}
