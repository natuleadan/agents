//! PostgreSQL pool management, embedded SQL migrations, and Ironic readiness probes.

use std::{future::Future, pin::Pin, sync::Arc};

use ironic::{HealthIndicator, HealthStatus};
use sqlx::{PgPool, postgres::PgPoolOptions};

use crate::infra::{bus::MessageBus, error::ServerError, state::StateBackend};

/// PostgreSQL connection pool and SQLx migration runner.
#[derive(Clone)]
pub struct Database {
    pool: PgPool,
}

impl Database {
    /// Connects to PostgreSQL and executes embedded migrations from `migrations/`.
    ///
    /// # Errors
    ///
    /// Returns [`ServerError::Internal`] when connection or migration fails.
    pub async fn connect_and_migrate(
        database_url: &str,
        max_connections: u32,
    ) -> Result<Self, ServerError> {
        let pool = PgPoolOptions::new()
            .max_connections(max_connections.max(1))
            .connect(database_url)
            .await
            .map_err(|e| ServerError::Internal(format!("postgres connect failed: {e}")))?;

        sqlx::migrate!("./migrations")
            .run(&pool)
            .await
            .map_err(|e| ServerError::Internal(format!("postgres migration failed: {e}")))?;

        Ok(Self { pool })
    }

    #[must_use]
    pub const fn pool(&self) -> &PgPool {
        &self.pool
    }

    /// Executes a lightweight connectivity check (`SELECT 1`).
    ///
    /// # Errors
    ///
    /// Returns [`ServerError::Internal`] when PostgreSQL is unreachable.
    pub async fn ping(&self) -> Result<(), ServerError> {
        sqlx::query("SELECT 1")
            .execute(&self.pool)
            .await
            .map_err(|e| ServerError::Internal(format!("postgres ping failed: {e}")))?;
        Ok(())
    }
}

/// Ironic readiness indicator for Valkey / ephemeral state store.
pub struct StateHealthIndicator {
    backend: Arc<dyn StateBackend>,
}

impl StateHealthIndicator {
    #[must_use]
    pub fn new(backend: Arc<dyn StateBackend>) -> Self {
        Self { backend }
    }
}

impl HealthIndicator for StateHealthIndicator {
    fn name(&self) -> &'static str {
        "valkey"
    }

    fn check(&self) -> Pin<Box<dyn Future<Output = HealthStatus> + Send + '_>> {
        self.check_readiness()
    }

    fn check_readiness(&self) -> Pin<Box<dyn Future<Output = HealthStatus> + Send + '_>> {
        Box::pin(async move {
            match self.backend.ping().await {
                Ok(()) => HealthStatus::Ok,
                Err(err) => HealthStatus::Unhealthy {
                    error: err.to_string(),
                },
            }
        })
    }
}

/// Ironic readiness indicator for NATS JetStream message bus.
pub struct BusHealthIndicator {
    bus: Arc<dyn MessageBus>,
}

impl BusHealthIndicator {
    #[must_use]
    pub fn new(bus: Arc<dyn MessageBus>) -> Self {
        Self { bus }
    }
}

impl HealthIndicator for BusHealthIndicator {
    fn name(&self) -> &'static str {
        "nats"
    }

    fn check(&self) -> Pin<Box<dyn Future<Output = HealthStatus> + Send + '_>> {
        self.check_readiness()
    }

    fn check_readiness(&self) -> Pin<Box<dyn Future<Output = HealthStatus> + Send + '_>> {
        Box::pin(async move {
            match self.bus.ping().await {
                Ok(()) => HealthStatus::Ok,
                Err(err) => HealthStatus::Unhealthy {
                    error: err.to_string(),
                },
            }
        })
    }
}

/// Ironic readiness indicator for PostgreSQL pool (optional when running in-memory).
pub struct PostgresHealthIndicator {
    db: Option<Database>,
}

impl PostgresHealthIndicator {
    #[must_use]
    pub const fn new(db: Option<Database>) -> Self {
        Self { db }
    }
}

impl HealthIndicator for PostgresHealthIndicator {
    fn name(&self) -> &'static str {
        "postgres"
    }

    fn check(&self) -> Pin<Box<dyn Future<Output = HealthStatus> + Send + '_>> {
        Box::pin(async move {
            match &self.db {
                Some(db) => match db.ping().await {
                    Ok(()) => HealthStatus::Ok,
                    Err(err) => HealthStatus::Unhealthy {
                        error: err.to_string(),
                    },
                },
                None => HealthStatus::Degraded {
                    message: Some("in-memory mode".to_owned()),
                },
            }
        })
    }

    fn check_readiness(&self) -> Pin<Box<dyn Future<Output = HealthStatus> + Send + '_>> {
        Box::pin(async move {
            match &self.db {
                Some(db) => match db.ping().await {
                    Ok(()) => HealthStatus::Ok,
                    Err(err) => HealthStatus::Unhealthy {
                        error: err.to_string(),
                    },
                },
                None => HealthStatus::Ok,
            }
        })
    }
}
