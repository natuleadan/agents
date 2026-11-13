//! Diagnostic preflight doctor for validating sovereign VPS and cloud-native topologies.

use std::{
    path::Path,
    time::{Duration, Instant},
};

use serde::Serialize;

use crate::infra::config::{EnvironmentMode, InfrastructureMode, ServerConfig};

/// Status outcome for an individual preflight check.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum CheckStatus {
    Pass,
    Fail,
    Skip,
}

impl CheckStatus {
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Pass => "[PASS]",
            Self::Fail => "[FAIL]",
            Self::Skip => "[SKIP]",
        }
    }
}

/// An individual preflight diagnostic outcome.
#[derive(Debug, Clone, Serialize)]
pub struct DoctorItem {
    pub name: String,
    pub status: CheckStatus,
    pub message: String,
    pub latency_ms: Option<u64>,
}

/// Aggregate doctor diagnostic report.
#[derive(Debug, Clone, Serialize)]
pub struct DoctorReport {
    pub mode: InfrastructureMode,
    pub env: EnvironmentMode,
    pub items: Vec<DoctorItem>,
    pub healthy: bool,
}

impl DoctorReport {
    /// Executes all preflight checks according to the configured topology and environment.
    pub async fn run(config: &ServerConfig) -> Self {
        let mut items = Vec::new();

        items.push(check_database(config).await);
        items.push(check_state_store(config).await);
        items.push(check_message_bus(config).await);
        items.push(check_sandbox_runtime(config).await);
        items.push(check_inference_providers(config));
        items.push(check_auxiliary_cloud(config));

        let healthy = items
            .iter()
            .all(|item| item.status == CheckStatus::Pass || item.status == CheckStatus::Skip);

        Self {
            mode: config.infrastructure_mode,
            env: config.env,
            items,
            healthy,
        }
    }

    /// Renders the diagnostic report to stdout with formatted columns.
    pub fn print_cli(&self) {
        println!(
            "================================================================================"
        );
        println!("AGENTS-SERVER DOCTOR: PREFLIGHT TOPOLOGY REPORT");
        println!(
            "================================================================================"
        );
        let mode_desc = if self.mode.is_managed_cloud() {
            "Topología B (Cloud-Native VPS Ligero: Upstash / Neon / Supabase)"
        } else {
            "Topología A (Self-Hosted / Soberano en VPS: Postgres / Valkey / NATS)"
        };
        println!("Operating Topology : {mode_desc}");
        println!("Environment Profile: {:?}", self.env);
        println!(
            "--------------------------------------------------------------------------------"
        );

        for item in &self.items {
            let latency_info = item
                .latency_ms
                .map(|ms| format!(" ({ms}ms)"))
                .unwrap_or_default();
            println!(
                "{:<8} {:<22} - {}{}",
                item.status.label(),
                item.name,
                item.message,
                latency_info
            );
        }

        println!(
            "--------------------------------------------------------------------------------"
        );
        if self.healthy {
            println!("Result: ALL CRITICAL PREFLIGHT CHECKS PASSED. Ready for traffic.");
        } else {
            println!(
                "Result: PREFLIGHT FAILED. One or more critical dependencies are unreachable."
            );
        }
        println!(
            "================================================================================"
        );
    }
}

async fn check_database(config: &ServerConfig) -> DoctorItem {
    let Some(ref db_url) = config.database_url else {
        return if config.env.is_production() || config.infrastructure_mode.is_managed_cloud() {
            DoctorItem {
                name: "Database".to_owned(),
                status: CheckStatus::Fail,
                message: "DATABASE_URL is not set".to_owned(),
                latency_ms: None,
            }
        } else {
            DoctorItem {
                name: "Database".to_owned(),
                status: CheckStatus::Pass,
                message: "Ephemeral in-memory fallback active (dev/test)".to_owned(),
                latency_ms: None,
            }
        };
    };

    let start = Instant::now();
    match sqlx::postgres::PgPoolOptions::new()
        .max_connections(1)
        .acquire_timeout(Duration::from_secs(3))
        .connect(db_url)
        .await
    {
        Ok(pool) => {
            let latency = u64::try_from(start.elapsed().as_millis()).unwrap_or(0);
            let migration_count: i64 =
                sqlx::query_scalar("SELECT count(*) FROM _sqlx_migrations WHERE success = true")
                    .fetch_one(&pool)
                    .await
                    .unwrap_or(0);

            DoctorItem {
                name: "Database (Postgres)".to_owned(),
                status: CheckStatus::Pass,
                message: format!("Connected successfully; {migration_count} migrations applied"),
                latency_ms: Some(latency),
            }
        }
        Err(err) => DoctorItem {
            name: "Database (Postgres)".to_owned(),
            status: CheckStatus::Fail,
            message: format!("Connection failed: {err}"),
            latency_ms: Some(u64::try_from(start.elapsed().as_millis()).unwrap_or(0)),
        },
    }
}

async fn check_state_store(config: &ServerConfig) -> DoctorItem {
    let _ = rustls::crypto::ring::default_provider().install_default();
    if config.infrastructure_mode.is_managed_cloud() {
        let Some(ref redis_url) = config.upstash_redis_url else {
            return DoctorItem {
                name: "Cache & State".to_owned(),
                status: CheckStatus::Fail,
                message: "REDIS_URL is required for managed-cloud mode".to_owned(),
                latency_ms: None,
            };
        };

        let start = Instant::now();
        match redis::Client::open(redis_url.as_str()) {
            Ok(client) => match client.get_connection_with_timeout(Duration::from_secs(3)) {
                Ok(mut conn) => match redis::cmd("PING").query::<String>(&mut conn) {
                    Ok(_) => DoctorItem {
                        name: "State (Upstash Redis)".to_owned(),
                        status: CheckStatus::Pass,
                        message: "TLS Redis connected and PING responded".to_owned(),
                        latency_ms: Some(u64::try_from(start.elapsed().as_millis()).unwrap_or(0)),
                    },
                    Err(err) => DoctorItem {
                        name: "State (Upstash Redis)".to_owned(),
                        status: CheckStatus::Fail,
                        message: format!("PING failed: {err}"),
                        latency_ms: Some(u64::try_from(start.elapsed().as_millis()).unwrap_or(0)),
                    },
                },
                Err(err) => DoctorItem {
                    name: "State (Upstash Redis)".to_owned(),
                    status: CheckStatus::Fail,
                    message: format!("Connection failed: {err}"),
                    latency_ms: Some(u64::try_from(start.elapsed().as_millis()).unwrap_or(0)),
                },
            },
            Err(err) => DoctorItem {
                name: "State (Upstash Redis)".to_owned(),
                status: CheckStatus::Fail,
                message: format!("Invalid Redis URL: {err}"),
                latency_ms: None,
            },
        }
    } else {
        let Some(ref valkey_url) = config.valkey_url else {
            return if config.env.is_production() {
                DoctorItem {
                    name: "Cache & State".to_owned(),
                    status: CheckStatus::Fail,
                    message: "VALKEY_URL is required in sovereign production".to_owned(),
                    latency_ms: None,
                }
            } else {
                DoctorItem {
                    name: "Cache & State".to_owned(),
                    status: CheckStatus::Pass,
                    message: "InMemoryStateBackend active (dev/test)".to_owned(),
                    latency_ms: None,
                }
            };
        };

        let start = Instant::now();
        match redis::Client::open(valkey_url.as_str()) {
            Ok(client) => match client.get_connection_with_timeout(Duration::from_secs(3)) {
                Ok(mut conn) => match redis::cmd("PING").query::<String>(&mut conn) {
                    Ok(_) => DoctorItem {
                        name: "State (Valkey)".to_owned(),
                        status: CheckStatus::Pass,
                        message: "Local Valkey connected and PING responded".to_owned(),
                        latency_ms: Some(u64::try_from(start.elapsed().as_millis()).unwrap_or(0)),
                    },
                    Err(err) => DoctorItem {
                        name: "State (Valkey)".to_owned(),
                        status: CheckStatus::Fail,
                        message: format!("PING failed: {err}"),
                        latency_ms: Some(u64::try_from(start.elapsed().as_millis()).unwrap_or(0)),
                    },
                },
                Err(err) => DoctorItem {
                    name: "State (Valkey)".to_owned(),
                    status: CheckStatus::Fail,
                    message: format!("Connection failed: {err}"),
                    latency_ms: Some(u64::try_from(start.elapsed().as_millis()).unwrap_or(0)),
                },
            },
            Err(err) => DoctorItem {
                name: "State (Valkey)".to_owned(),
                status: CheckStatus::Fail,
                message: format!("Invalid Valkey URL: {err}"),
                latency_ms: None,
            },
        }
    }
}

async fn check_message_bus(config: &ServerConfig) -> DoctorItem {
    if config.infrastructure_mode.is_managed_cloud() {
        if config.qstash_url.is_some() && config.qstash_token.is_some() {
            DoctorItem {
                name: "Message Bus (QStash)".to_owned(),
                status: CheckStatus::Pass,
                message: "QStash REST event bus and authentication configured".to_owned(),
                latency_ms: None,
            }
        } else {
            DoctorItem {
                name: "Message Bus (QStash)".to_owned(),
                status: CheckStatus::Fail,
                message: "QSTASH_URL and QSTASH_TOKEN are required in cloud mode".to_owned(),
                latency_ms: None,
            }
        }
    } else {
        let Some(ref nats_url) = config.nats_url else {
            return if config.env.is_production() {
                DoctorItem {
                    name: "Message Bus".to_owned(),
                    status: CheckStatus::Fail,
                    message: "NATS_URL is required in sovereign production".to_owned(),
                    latency_ms: None,
                }
            } else {
                DoctorItem {
                    name: "Message Bus".to_owned(),
                    status: CheckStatus::Pass,
                    message: "InMemoryMessageBus active (dev/test)".to_owned(),
                    latency_ms: None,
                }
            };
        };

        let start = Instant::now();
        match tokio::time::timeout(
            Duration::from_secs(3),
            async_nats::connect(nats_url.as_str()),
        )
        .await
        {
            Ok(Ok(_client)) => DoctorItem {
                name: "Message Bus (NATS)".to_owned(),
                status: CheckStatus::Pass,
                message: "NATS JetStream connected".to_owned(),
                latency_ms: Some(u64::try_from(start.elapsed().as_millis()).unwrap_or(0)),
            },
            Ok(Err(err)) => DoctorItem {
                name: "Message Bus (NATS)".to_owned(),
                status: CheckStatus::Fail,
                message: format!("NATS connection error: {err}"),
                latency_ms: Some(u64::try_from(start.elapsed().as_millis()).unwrap_or(0)),
            },
            Err(_) => DoctorItem {
                name: "Message Bus (NATS)".to_owned(),
                status: CheckStatus::Fail,
                message: "NATS connection timed out after 3s".to_owned(),
                latency_ms: Some(3000),
            },
        }
    }
}

async fn check_sandbox_runtime(config: &ServerConfig) -> DoctorItem {
    if config.infrastructure_mode.is_managed_cloud() {
        if config.upstash_box_token.is_some() || config.deepinfra_token.is_some() {
            DoctorItem {
                name: "Sandbox Runtime".to_owned(),
                status: CheckStatus::Pass,
                message: "Cloud MicroVM providers (Upstash Box / DeepInfra) configured".to_owned(),
                latency_ms: None,
            }
        } else {
            DoctorItem {
                name: "Sandbox Runtime".to_owned(),
                status: CheckStatus::Fail,
                message: "Missing UPSTASH_BOX_TOKEN or DEEPINFRA_TOKEN for cloud sandboxes"
                    .to_owned(),
                latency_ms: None,
            }
        }
    } else {
        let docker_sock = Path::new("/var/run/docker.sock").exists();
        let docker_host = std::env::var("DOCKER_HOST").is_ok();
        if docker_sock || docker_host {
            DoctorItem {
                name: "Sandbox Runtime".to_owned(),
                status: CheckStatus::Pass,
                message: "Docker socket available for sovereign container execution".to_owned(),
                latency_ms: None,
            }
        } else {
            DoctorItem {
                name: "Sandbox Runtime".to_owned(),
                status: CheckStatus::Pass,
                message: "agents-boxer host process execution ready".to_owned(),
                latency_ms: None,
            }
        }
    }
}

fn check_inference_providers(config: &ServerConfig) -> DoctorItem {
    let mut configured = Vec::new();
    if config.deepinfra_token.is_some() {
        configured.push("DeepInfra");
    }
    if config.deepseek_api_key.is_some() {
        configured.push("DeepSeek");
    }
    if config.ai_gateway_api_key.is_some() {
        configured.push("AI Gateway");
    }

    if configured.is_empty() {
        DoctorItem {
            name: "Inference Providers".to_owned(),
            status: CheckStatus::Skip,
            message: "No external keys configured; local Ollama/Mock inference expected".to_owned(),
            latency_ms: None,
        }
    } else {
        DoctorItem {
            name: "Inference Providers".to_owned(),
            status: CheckStatus::Pass,
            message: format!("Configured: {}", configured.join(", ")),
            latency_ms: None,
        }
    }
}

fn check_auxiliary_cloud(config: &ServerConfig) -> DoctorItem {
    let mut configured = Vec::new();
    if config.upstash_vector_rest_url.is_some() {
        configured.push("Upstash Vector");
    }
    if config.upstash_blob_token.is_some() {
        configured.push("Upstash Blob");
    }

    if configured.is_empty() {
        DoctorItem {
            name: "Auxiliary Cloud".to_owned(),
            status: CheckStatus::Skip,
            message: "No optional cloud auxiliary storage services configured".to_owned(),
            latency_ms: None,
        }
    } else {
        DoctorItem {
            name: "Auxiliary Cloud".to_owned(),
            status: CheckStatus::Pass,
            message: format!("Active: {}", configured.join(", ")),
            latency_ms: None,
        }
    }
}
