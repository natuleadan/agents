//! Background workers for maintenance, retention, and scheduled tasks.

use std::time::Duration;

use ironic::{Application, OpenApiAxumApplication};

use crate::infra::error::ServerError;

/// Spawns a periodic background worker enforcing [`AuditService::prune_expired`] retention.
///
/// # Errors
///
/// Returns [`ServerError`] when [`AuditService`] cannot be resolved from the container.
pub async fn start_retention_worker(
    app: &Application<OpenApiAxumApplication>,
    interval: Duration,
) -> Result<tokio::task::JoinHandle<()>, ServerError> {
    let audit = app
        .container()
        .resolve::<crate::modules::audit::services::AuditService>()
        .await
        .map_err(|e| ServerError::Internal(e.to_string()))?;

    let handle = tokio::spawn(async move {
        let mut ticker = tokio::time::interval(interval);
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        ticker.tick().await;
        loop {
            ticker.tick().await;
            match audit.prune_expired().await {
                Ok(pruned) if pruned > 0 => {
                    tracing::info!(pruned, "audit retention worker archived expired entries");
                }
                Ok(_) => {}
                Err(err) => {
                    tracing::warn!(error = %err, "audit retention worker iteration failed");
                }
            }
        }
    });
    Ok(handle)
}
