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
use tracing::info;
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let _ = dotenvy::dotenv();
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();

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
