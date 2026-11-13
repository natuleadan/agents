//! Boxer worker daemon CLI entrypoint.

#![forbid(unsafe_code)]

use std::sync::Arc;

use agents_boxer::{
    config::{BoxerCli, BoxerConfig},
    daemon::{BoxerDaemon, NatsBoxerBus},
    leases::HttpServerLeaseClient,
};
use clap::Parser;
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
    let cfg = BoxerConfig::from_cli(cli)?;
    let lease_client = Arc::new(HttpServerLeaseClient::new(
        cfg.server_internal_url.clone(),
        cfg.worker_token.clone(),
    ));
    let nats_bus = NatsBoxerBus::connect(&cfg).await?;
    let daemon = Arc::new(BoxerDaemon::new(cfg, lease_client, nats_bus.clone())?);
    daemon.run_nats_until_shutdown(&nats_bus).await?;
    Ok(())
}
