//! `agents-migrate` standalone CLI for zero-downtime database export, import, and verification.

#![forbid(unsafe_code)]

use std::{fs, path::PathBuf};

use agents_server::{
    MigrationSnapshot, ServerConfig, export_snapshot, import_snapshot, verify_database,
};
use clap::{Parser, Subcommand};
use sqlx::postgres::PgPoolOptions;

#[derive(Debug, Parser)]
#[command(
    name = "agents-migrate",
    version,
    about = "Data migration engine between Topología A (Self-Hosted) and Topología B (Cloud-Native)"
)]
struct Cli {
    #[command(subcommand)]
    command: MigrateSubcommand,
}

#[derive(Debug, Subcommand)]
pub enum MigrateSubcommand {
    /// Export database tables and tamper-evident audit logs to a JSON snapshot.
    Export {
        /// Target JSON snapshot file path.
        #[arg(long, short = 'o')]
        out: PathBuf,
        /// Database connection URL (defaults to DATABASE_URL environment variable).
        #[arg(long)]
        db_url: Option<String>,
    },
    /// Import a JSON snapshot into a destination PostgreSQL/Neon/Supabase database.
    Import {
        /// Source JSON snapshot file path.
        #[arg(long, short = 'i', name = "in")]
        in_file: PathBuf,
        /// Target database connection URL.
        #[arg(long)]
        target_url: Option<String>,
    },
    /// Verify schema migrations and audit log SHA-256 hash chain in a database.
    Verify {
        /// Target database connection URL (defaults to DATABASE_URL environment variable).
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
    match cli.command {
        MigrateSubcommand::Export { out, db_url } => {
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

            let json = serde_json::to_string_pretty(&snapshot)?;
            fs::write(&out, json)?;
            println!(
                "[PASS] Successfully exported snapshot to {:?} (users: {}, workflows: {}, audit_logs: {})",
                out,
                snapshot.users.len(),
                snapshot.workflows.len(),
                snapshot.audit_logs.len()
            );
        }
        MigrateSubcommand::Import {
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
        MigrateSubcommand::Verify { db_url } => {
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
    }

    Ok(())
}
