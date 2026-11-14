//! Boxer worker node autodiscovery and periodic heartbeats to agents-server.

use std::{sync::Arc, time::Duration};

use ironic::services::scheduling::{ScheduledTask, interval};
use sha2::{Digest, Sha256};
use tracing::{debug, warn};

use crate::{
    config::{BoxerConfig, now_ms},
    control_plane::service::BoxerControlPlaneService,
};

fn compute_hmac_hex(key: &[u8], msg: &[u8]) -> String {
    const BLOCK_SIZE: usize = 64;
    let mut k_block = [0u8; BLOCK_SIZE];
    if key.len() > BLOCK_SIZE {
        let hashed = Sha256::digest(key);
        k_block[..32].copy_from_slice(&hashed);
    } else {
        k_block[..key.len()].copy_from_slice(key);
    }
    let mut o_pad = [0x5cu8; BLOCK_SIZE];
    let mut i_pad = [0x36u8; BLOCK_SIZE];
    for i in 0..BLOCK_SIZE {
        o_pad[i] ^= k_block[i];
        i_pad[i] ^= k_block[i];
    }
    let mut inner = Sha256::new();
    inner.update(i_pad);
    inner.update(msg);
    let inner_hash = inner.finalize();

    let mut outer = Sha256::new();
    outer.update(o_pad);
    outer.update(inner_hash);
    hex::encode(outer.finalize())
}

static HEARTBEAT_SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// Spawns an Ironic-native periodic task that announces Boxer capacity and node status to agents-server
/// via HMAC-SHA256 signed `POST /api/v1/nodes/register`.
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
            let seq = HEARTBEAT_SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
            let health = svc.health().await;
            let register_url = format!("{}/api/v1/nodes/register", cfg.server_internal_url);
            let node_ip = "127.0.0.1";
            let timestamp_ms = now_ms();
            let sign_payload =
                format!("{}:{node_ip}:{}:{timestamp_ms}", cfg.node_id, cfg.http_port);
            let signature =
                compute_hmac_hex(cfg.cluster_secret.as_bytes(), sign_payload.as_bytes());

            let payload = serde_json::json!({
                "nodeId": cfg.node_id,
                "nodeIp": node_ip,
                "port": cfg.http_port,
                "runtime": cfg.default_profile.runtime,
                "isolation": cfg.default_profile.isolation,
                "maxConcurrentBoxes": cfg.max_concurrent_boxes,
                "activeBoxes": health.active_boxes,
                "timestampMs": timestamp_ms,
                "signature": signature,
            });

            let traceparent = format!(
                "00-{:032x}-{:016x}-01",
                uuid::Uuid::new_v4().as_u128(),
                (uuid::Uuid::new_v4().as_u128() >> 64) as u64
            );

            match http
                .post(&register_url)
                .header("Content-Type", "application/json")
                .header("traceparent", &traceparent)
                .json(&payload)
                .send()
                .await
            {
                Ok(resp) if resp.status().is_success() => {
                    if seq == 1 || seq.is_multiple_of(10) {
                        tracing::info!(
                            node_id = %cfg.node_id,
                            seq,
                            active_boxes = health.active_boxes,
                            traceparent = %traceparent,
                            "sent signed Boxer node registration heartbeat"
                        );
                    } else {
                        debug!(
                            node_id = %cfg.node_id,
                            seq,
                            active_boxes = health.active_boxes,
                            "sent signed Boxer node registration heartbeat"
                        );
                    }
                }
                Ok(resp) => {
                    warn!(
                        node_id = %cfg.node_id,
                        status = %resp.status(),
                        "Boxer signed registration heartbeat rejected by agents-server"
                    );
                }
                Err(err) => {
                    debug!(
                        node_id = %cfg.node_id,
                        error = %err,
                        "autodiscovery heartbeat could not reach agents-server (retrying next tick)"
                    );
                }
            }
        }
    })
}
