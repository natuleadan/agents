//! Domain entities for registered VLAN Boxer nodes and distributed preview routes.

use agents_protocol::{BoxerNodeAnnouncement, PreviewRouteSync};
use serde::{Deserialize, Serialize};

/// Snapshot of active auto-registered VLAN worker nodes.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NodeListResponse {
    pub nodes: Vec<BoxerNodeAnnouncement>,
    pub total: usize,
}

/// Acknowledgment returned after syncing a dynamic preview route.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PreviewSyncAck {
    pub route_id: String,
    pub active: bool,
    pub active_routes: usize,
}

/// Stored distributed preview entry tracked by the node registry.
#[derive(Debug, Clone)]
pub struct StoredPreviewEntry {
    pub sync: PreviewRouteSync,
}
