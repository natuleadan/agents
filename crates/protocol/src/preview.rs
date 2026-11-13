//! Ephemeral reverse-proxy preview route contracts for Boxer web deliverables.

use serde::{Deserialize, Serialize};

/// Dynamic Traefik route published over private VLAN for temporary app previews.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TraefikPreviewRoute {
    pub route_id: String,
    pub task_id: String,
    pub slug: String,
    pub internal_host: String,
    pub internal_port: u16,
    pub public_url: String,
    pub ttl_seconds: u64,
    pub requires_token: bool,
    pub created_at_ms: u64,
}
