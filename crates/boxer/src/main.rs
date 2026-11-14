//! Boxer worker daemon CLI entrypoint.

#![forbid(unsafe_code)]

use std::{sync::Arc, time::Duration};

use agents_boxer::{
    config::{BoxerCli, BoxerConfig},
    control_plane::{
        BoxerControlPlaneService, build_control_plane_router, start_autodiscovery_heartbeat,
    },
    daemon::{BoxerDaemon, NatsBoxerBus},
    leases::HttpServerLeaseClient,
};
use clap::Parser;
use ironic::logging::{FileLogStorage, TimeSeriesConfig};
use tracing::info;
use tracing_subscriber::{EnvFilter, Layer, layer::SubscriberExt, util::SubscriberInitExt};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let _ = dotenvy::dotenv();
    let boxer_log_dir = std::env::var("AGENTS_BOXER_LOG_DIR")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|_| {
            if let Ok(base) = std::env::var("AGENTS_LOG_DIR") {
                std::path::PathBuf::from(base).join("boxer")
            } else if std::path::Path::new("/var/log/agents").is_dir() {
                std::path::PathBuf::from("/var/log/agents/boxer")
            } else {
                std::path::PathBuf::from(".logs/boxer")
            }
        });
    let _ = std::fs::create_dir_all(&boxer_log_dir);
    let ts_layer = TimeSeriesConfig::default()
        .with_storage(Arc::new(FileLogStorage::new(&boxer_log_dir)))
        .layer();
    let ts_filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    let fmt_filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    let _ = tracing_subscriber::registry()
        .with(ts_layer.with_filter(ts_filter))
        .with(tracing_subscriber::fmt::layer().with_filter(fmt_filter))
        .try_init();

    let cli = BoxerCli::parse();
    let cfg = Arc::new(BoxerConfig::from_cli(cli)?);
    let lease_client = Arc::new(HttpServerLeaseClient::new(
        cfg.server_internal_url.clone(),
        cfg.worker_token.clone(),
    ));
    let nats_bus = NatsBoxerBus::connect(&cfg).await?;
    let daemon = Arc::new(BoxerDaemon::new(
        cfg.as_ref().clone(),
        lease_client,
        nats_bus.clone(),
    )?);

    let cp_service = Arc::new(BoxerControlPlaneService::new(Arc::clone(&cfg)));
    let app = build_control_plane_router(Arc::clone(&cp_service), &cfg);

    let bind_addr = std::net::SocketAddr::from(([0, 0, 0, 0], cfg.http_port));
    info!("Starting Boxer HTTP control plane on {}", bind_addr);
    let listener = tokio::net::TcpListener::bind(bind_addr).await?;

    let _heartbeat_task = start_autodiscovery_heartbeat(
        Arc::clone(&cfg),
        Arc::clone(&cp_service),
        Duration::from_secs(30),
    );

    tokio::select! {
        res = daemon.run_nats_until_shutdown(&nats_bus) => {
            res?;
        }
        res = axum::serve(listener, app) => {
            res?;
        }
    }
    Ok(())
}
