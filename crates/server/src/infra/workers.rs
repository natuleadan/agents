//! Background workers for maintenance, retention, and scheduled tasks.

use std::{sync::Arc, time::Duration};

use ironic::{
    Application, OpenApiAxumApplication,
    services::scheduling::{ScheduledTask, interval},
};

use crate::infra::error::ServerError;

/// Spawns a periodic background worker enforcing [`AuditService::prune_expired`] retention
/// via Ironic's cooperative [`ScheduledTask`] scheduler.
///
/// # Errors
///
/// Returns [`ServerError`] when [`AuditService`] cannot be resolved from the container.
pub async fn start_retention_worker(
    app: &Application<OpenApiAxumApplication>,
    period: Duration,
) -> Result<ScheduledTask, ServerError> {
    let audit = app
        .container()
        .resolve::<crate::modules::audit::services::AuditService>()
        .await
        .map_err(|e| ServerError::Internal(e.to_string()))?;

    let task = interval(period, move || {
        let audit = Arc::clone(&audit);
        async move {
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
    Ok(task)
}

/// Runs boot recovery for interrupted workflow runs and starts the cooperative autocron runner.
///
/// # Errors
///
/// Returns [`ServerError`] when [`AutocronService`] cannot be resolved from the container.
pub async fn start_autocron_worker(
    app: &Application<OpenApiAxumApplication>,
    period: Duration,
) -> Result<ScheduledTask, ServerError> {
    let autocron = app
        .container()
        .resolve::<crate::modules::autocron::services::AutocronService>()
        .await
        .map_err(|e| ServerError::Internal(e.to_string()))?;

    let boot_id = uuid::Uuid::new_v4().to_string();
    match autocron.recover_on_boot(&boot_id).await {
        Ok(report) => {
            tracing::info!(
                boot_id = %report.boot_id,
                resumed_runs = report.resumed_runs,
                triggered_due_schedules = report.triggered_due_schedules,
                "autocron boot recovery completed"
            );
        }
        Err(err) => {
            tracing::warn!(boot_id = %boot_id, error = %err, "autocron boot recovery failed");
        }
    }

    Ok(autocron.start_scheduled_runner(period, boot_id))
}
