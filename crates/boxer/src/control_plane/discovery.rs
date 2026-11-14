//! Boxer worker node autodiscovery and periodic heartbeats to agents-server.

use std::{sync::Arc, time::Duration};

use ironic::services::scheduling::{ScheduledTask, interval};
use tracing::debug;

use crate::{config::BoxerConfig, control_plane::service::BoxerControlPlaneService};

/// Spawns an Ironic-native periodic task that announces Boxer capacity and node status to agents-server.
pub fn start_autodiscovery_heartbeat(
    config: Arc<BoxerConfig>,
    service: Arc<BoxerControlPlaneService>,
    period: Duration,
) -> ScheduledTask {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(5))
        .build()
        .unwrap_or_default();

    interval(period, move || {
        let cfg = Arc::clone(&config);
        let svc = Arc::clone(&service);
        let http = client.clone();

        async move {
            let health = svc.health().await;
            let heartbeat_url = format!("{}/api/v1/nodes/heartbeat", cfg.server_internal_url);

            let payload = serde_json::json!({
                "nodeId": cfg.node_id,
                "vlanIp": "127.0.0.1",
                "httpPort": cfg.http_port,
                "capacityBoxes": cfg.max_concurrent_boxes as u32,
                "activeBoxes": health.active_boxes as u32,
                "capabilities": ["docker", "openshell", "vfs"],
                "status": "active"
            });

            match http
                .post(&heartbeat_url)
                .header("Authorization", format!("Bearer {}", cfg.worker_token))
                .header("Content-Type", "application/json")
                .json(&payload)
                .send()
                .await
            {
                Ok(resp) if resp.status().is_success() => {
                    debug!("node heartbeat acknowledged by server: {}", cfg.node_id);
                }
                Ok(resp) => {
                    debug!(
                        "node heartbeat returned non-200: {} ({})",
                        resp.status(),
                        cfg.node_id
                    );
                }
                Err(err) => {
                    debug!("failed to send node heartbeat to {}: {err}", heartbeat_url);
                }
            }
        }
    })
}
