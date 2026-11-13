//! `agents-server` daemon entry point (`serve`, `migrate`, `schema`).

#![forbid(unsafe_code)]

use std::time::Duration;

use agents_protocol::export_protocol_schema;
use agents_server::{
    InfrastructureHandles, ServerConfig, app::start_retention_worker, build_application,
};
use clap::{Parser, Subcommand};

#[derive(Debug, Parser)]
#[command(
    name = "agents-server",
    version,
    about = "Distributed agent harness orchestrator daemon"
)]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Start the HTTP/WebSocket/SSE/MCP orchestrator daemon.
    Serve,
    /// Connect to PostgreSQL, apply SQL migrations, and exit cleanly.
    Migrate,
    /// Print the canonical protocol schema bundle JSON to stdout.
    Schema,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    dotenvy::dotenv().ok();
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    let cli = Cli::parse();
    match cli.command.unwrap_or(Command::Serve) {
        Command::Schema => {
            let bundle = export_protocol_schema();
            println!("{}", serde_json::to_string_pretty(&bundle)?);
        }
        Command::Migrate => {
            let config = ServerConfig::from_env()?;
            let _infra = InfrastructureHandles::from_config(config).await?;
            tracing::info!("database migrations applied cleanly");
        }
        Command::Serve => {
            let config = ServerConfig::from_env()?;
            let bind_addr = config.bind_addr.clone();
            let infra = InfrastructureHandles::from_config(config).await?;
            let app = build_application(infra).await?;
            let retention_handle = start_retention_worker(&app, Duration::from_secs(3600)).await?;
            tracing::info!("agents-server listening on http://{bind_addr}");
            let result = app.listen(&bind_addr).await;
            retention_handle.abort();
            result?;
        }
    }
    Ok(())
}
