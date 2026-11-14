//! `agents-server` daemon entry point (`serve`, `migrate`, `doctor`, `schema`).

#![forbid(unsafe_code)]

use std::{fs, path::PathBuf, time::Duration};

use agents_protocol::export_protocol_schema;
use agents_server::{
    DoctorReport, InfrastructureHandles, MigrationSnapshot, ServerConfig,
    app::{start_autocron_worker, start_retention_worker},
    build_application, export_snapshot, import_snapshot, verify_database,
};
use clap::{Parser, Subcommand};
use sqlx::postgres::PgPoolOptions;

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
    /// Connect to PostgreSQL, apply SQL migrations, or run data migrations.
    Migrate {
        #[command(subcommand)]
        action: Option<MigrateAction>,
    },
    /// Run diagnostic preflight checks on the active operating mode.
    Doctor,
    /// Print the canonical protocol schema bundle JSON to stdout.
    Schema,
}

#[derive(Debug, Subcommand)]
enum MigrateAction {
    /// Apply pending SQL migrations (default).
    Up,
    /// Export database tables and tamper-evident audit logs to a JSON snapshot.
    Export {
        #[arg(long, short = 'o')]
        out: PathBuf,
        #[arg(long)]
        db_url: Option<String>,
    },
    /// Import a JSON snapshot into target PostgreSQL/Neon/Supabase database.
    Import {
        #[arg(long, short = 'i', name = "in")]
        in_file: PathBuf,
        #[arg(long)]
        target_url: Option<String>,
    },
    /// Verify database migrations and audit log SHA-256 hash chain integrity.
    Verify {
        #[arg(long)]
        db_url: Option<String>,
    },
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
        Command::Doctor => {
            let config = ServerConfig::from_env().unwrap_or_else(|err| {
                eprintln!("[FAIL] Configuration error: {err}");
                std::process::exit(1);
            });
            let report = DoctorReport::run(&config).await;
            report.print_cli();
            if !report.healthy {
                std::process::exit(1);
            }
        }
        Command::Migrate { action } => match action.unwrap_or(MigrateAction::Up) {
            MigrateAction::Up => {
                let config = ServerConfig::from_env()?;
                let _infra = InfrastructureHandles::from_config(config).await?;
                tracing::info!("database migrations applied cleanly");
            }
            MigrateAction::Export { out, db_url } => {
                let config = ServerConfig::from_env().unwrap_or_else(|_| ServerConfig::for_tests());
                let url = db_url
                    .or(config.database_url)
                    .ok_or("DATABASE_URL must be specified or configured")?;
                let pool = PgPoolOptions::new()
                    .max_connections(2)
                    .connect(&url)
                    .await?;
                let mode_str = format!("{:?}", config.infrastructure_mode);
                let snapshot = export_snapshot(&pool, &mode_str).await?;
                fs::write(&out, serde_json::to_string_pretty(&snapshot)?)?;
                println!(
                    "[PASS] Successfully exported snapshot to {:?} (users: {}, workflows: {}, audit_logs: {})",
                    out,
                    snapshot.users.len(),
                    snapshot.workflows.len(),
                    snapshot.audit_logs.len()
                );
            }
            MigrateAction::Import {
                in_file,
                target_url,
            } => {
                let config = ServerConfig::from_env().unwrap_or_else(|_| ServerConfig::for_tests());
                let url = target_url
                    .or(config.database_url)
                    .ok_or("Target DATABASE_URL must be specified or configured")?;
                let raw = fs::read_to_string(&in_file)?;
                let snapshot: MigrationSnapshot = serde_json::from_str(&raw)?;
                let pool = PgPoolOptions::new()
                    .max_connections(2)
                    .connect(&url)
                    .await?;
                let summary = import_snapshot(&pool, &snapshot).await?;
                println!(
                    "[PASS] Restored snapshot into target database (users: {}, workflows: {}, audit_logs: {}, audit_chain_valid: {})",
                    summary.users_imported,
                    summary.workflows_imported,
                    summary.audit_logs_imported,
                    summary.audit_chain_valid
                );
            }
            MigrateAction::Verify { db_url } => {
                let config = ServerConfig::from_env().unwrap_or_else(|_| ServerConfig::for_tests());
                let url = db_url
                    .or(config.database_url)
                    .ok_or("DATABASE_URL must be specified or configured")?;
                let pool = PgPoolOptions::new()
                    .max_connections(2)
                    .connect(&url)
                    .await?;
                let report = verify_database(&pool).await?;
                if report.healthy {
                    println!("[PASS] {}", report.message);
                } else {
                    eprintln!("[FAIL] {}", report.message);
                    std::process::exit(1);
                }
            }
        },
        Command::Serve => {
            let config = ServerConfig::from_env()?;
            let bind_addr = config.bind_addr.clone();
            let infra = InfrastructureHandles::from_config(config).await?;
            let app = build_application(infra).await?;
            let retention_task = start_retention_worker(&app, Duration::from_secs(3600)).await?;
            let autocron_task = start_autocron_worker(&app, Duration::from_secs(15)).await?;
            tracing::info!("agents-server listening on http://{bind_addr}");
            let result = app.listen(&bind_addr).await;
            let _ = autocron_task.shutdown().await;
            let _ = retention_task.shutdown().await;
            result?;
        }
    }
    Ok(())
}
