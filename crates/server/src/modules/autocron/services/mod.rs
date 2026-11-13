//! Timezone-aware Autocron scheduler, budget ceiling enforcement, ambient event dispatch,
//! and startup crash-resume reconciliation.

pub mod cron;

use std::sync::Arc;

use agents_protocol::{StreamPayload, TriggerKind};
use ironic::prelude::*;
use serde_json::{Value, json};

use crate::{
    infra::{
        error::ServerError,
        state::now_ms,
        stream::{BusClient, StreamStore, emit_stream_event},
    },
    modules::{
        audit::services::AuditService,
        auth::{entities::AuthenticatedActor, guards::ensure_owner},
        autocron::{
            dto::{CreateScheduleDto, ExecuteScheduleDto, TriggerWebhookDto},
            entities::{AutocronRecoveryReport, AutocronScheduleEntity, AutocronTickOutcome},
            repositories::AutocronStore,
        },
        workflows::services::WorkflowService,
    },
};

/// Coordinates Autocron schedules, budget caps, webhooks, and startup recovery.
#[derive(Injectable)]
pub struct AutocronService {
    store: Arc<AutocronStore>,
    workflows: Arc<WorkflowService>,
    streams: Arc<StreamStore>,
    bus: Arc<BusClient>,
    audit: Arc<AuditService>,
}

impl AutocronService {
    /// Creates a new Autocron schedule bound to an owned workflow DAG.
    ///
    /// # Errors
    ///
    /// Returns [`ServerError::Validation`] on invalid cron/timezone or [`ServerError::Forbidden`] if workflow is unowned.
    pub async fn create_schedule(
        &self,
        actor: &AuthenticatedActor,
        dto: &CreateScheduleDto,
    ) -> Result<AutocronScheduleEntity, ServerError> {
        let _ = self.workflows.get_owned(actor, &dto.workflow_id).await?;
        let timezone = dto
            .timezone
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .unwrap_or("UTC")
            .to_owned();
        let now = now_ms();
        let enabled = dto.enabled.unwrap_or(true);
        let next_run_at = if enabled {
            cron::compute_next_run_ms(
                &dto.trigger,
                dto.cron_expr.as_deref(),
                dto.interval_seconds,
                &timezone,
                now,
            )?
        } else {
            let _ = cron::compute_next_run_ms(
                &dto.trigger,
                dto.cron_expr.as_deref(),
                dto.interval_seconds,
                &timezone,
                now,
            )?;
            None
        };

        let entity = AutocronScheduleEntity {
            schedule_id: uuid::Uuid::new_v4().to_string(),
            owner_id: actor.user_id.clone(),
            workflow_id: dto.workflow_id.clone(),
            trigger: dto.trigger.clone(),
            cron_expr: dto.cron_expr.clone(),
            interval_seconds: dto.interval_seconds,
            timezone,
            max_budget_usd_cents: dto.max_budget_usd_cents,
            spent_budget_usd_cents: 0,
            ambient_mode: dto.ambient_mode.unwrap_or(false),
            enabled,
            next_run_at,
            last_run_at: None,
            created_at: now,
            updated_at: now,
        };
        let saved = self.store.0.create_schedule(entity).await?;
        let _ = self
            .audit
            .record(
                Some(&actor.user_id),
                "autocron.create",
                "autocron_schedule",
                Some(&saved.schedule_id),
                "success",
                json!({
                    "workflowId": saved.workflow_id,
                    "trigger": saved.trigger.as_str(),
                    "timezone": saved.timezone,
                }),
            )
            .await;
        Ok(saved)
    }

    /// Lists all Autocron schedules belonging to `actor`.
    pub async fn list_owned(
        &self,
        actor: &AuthenticatedActor,
    ) -> Result<Vec<AutocronScheduleEntity>, ServerError> {
        self.store.0.list_schedules_for_owner(&actor.user_id).await
    }

    /// Loads an owned Autocron schedule with strict BOLA verification.
    pub async fn get_owned(
        &self,
        actor: &AuthenticatedActor,
        schedule_id: &str,
    ) -> Result<AutocronScheduleEntity, ServerError> {
        let sched = self
            .store
            .0
            .find_schedule(schedule_id)
            .await?
            .ok_or_else(|| {
                ServerError::NotFound(format!("autocron schedule `{schedule_id}` not found"))
            })?;
        ensure_owner(actor, &sched.owner_id).map_err(|_| {
            ServerError::Forbidden("Access denied: caller does not own target schedule".into())
        })?;
        Ok(sched)
    }

    /// Enables or disables an owned Autocron schedule.
    pub async fn toggle_owned(
        &self,
        actor: &AuthenticatedActor,
        schedule_id: &str,
        enabled: bool,
    ) -> Result<AutocronScheduleEntity, ServerError> {
        let _ = self.get_owned(actor, schedule_id).await?;
        self.store
            .0
            .set_enabled(schedule_id, enabled)
            .await?
            .ok_or_else(|| {
                ServerError::NotFound(format!("autocron schedule `{schedule_id}` not found"))
            })
    }

    /// Triggers an owned schedule explicitly with budget enforcement.
    pub async fn execute_owned(
        &self,
        actor: &AuthenticatedActor,
        schedule_id: &str,
        dto: &ExecuteScheduleDto,
    ) -> Result<AutocronTickOutcome, ServerError> {
        let sched = self.get_owned(actor, schedule_id).await?;
        let boot_id = dto
            .boot_id
            .as_deref()
            .filter(|s| !s.trim().is_empty())
            .unwrap_or("manual-cron");
        self.execute_schedule_internal(
            &sched,
            boot_id,
            dto.estimated_cost_cents.unwrap_or(0),
            dto.input.clone().unwrap_or(Value::Null),
            now_ms(),
        )
        .await
    }

    /// Triggers a webhook-configured schedule for an authorized caller.
    pub async fn trigger_webhook_owned(
        &self,
        actor: &AuthenticatedActor,
        schedule_id: &str,
        dto: &TriggerWebhookDto,
    ) -> Result<AutocronTickOutcome, ServerError> {
        let sched = self.get_owned(actor, schedule_id).await?;
        if sched.trigger != TriggerKind::Webhook {
            return Err(ServerError::Validation(format!(
                "schedule `{schedule_id}` does not accept webhook triggers"
            )));
        }
        self.execute_schedule_internal(
            &sched,
            "webhook",
            dto.estimated_cost_cents.unwrap_or(0),
            dto.payload.clone().unwrap_or(Value::Null),
            now_ms(),
        )
        .await
    }

    /// Dispatches due schedules up to `current_ms` while enforcing budget caps.
    pub async fn tick_due_schedules(
        &self,
        boot_id: &str,
        current_ms: u64,
    ) -> Result<u32, ServerError> {
        let due = self.store.0.list_due_schedules(current_ms).await?;
        let mut fired: u32 = 0;
        for sched in due {
            match self
                .execute_schedule_internal(&sched, boot_id, 0, Value::Null, current_ms)
                .await
            {
                Ok(_) => fired = fired.saturating_add(1),
                Err(ServerError::Conflict(_)) => {}
                Err(err) => return Err(err),
            }
        }
        Ok(fired)
    }

    /// Reconciles crashed/orphaned workflow runs from prior `boot_id`s and ticks due schedules.
    pub async fn recover_on_boot(
        &self,
        boot_id: &str,
    ) -> Result<AutocronRecoveryReport, ServerError> {
        let resumed_runs = self.workflows.reconcile_interrupted_runs(boot_id).await?;
        let triggered_due_schedules = self.tick_due_schedules(boot_id, now_ms()).await?;
        Ok(AutocronRecoveryReport {
            boot_id: boot_id.to_owned(),
            resumed_runs,
            triggered_due_schedules,
        })
    }

    async fn execute_schedule_internal(
        &self,
        sched: &AutocronScheduleEntity,
        boot_id: &str,
        estimated_cost_cents: u64,
        input: Value,
        now: u64,
    ) -> Result<AutocronTickOutcome, ServerError> {
        if !sched.enabled {
            return Err(ServerError::Conflict(format!(
                "autocron schedule `{}` is disabled",
                sched.schedule_id
            )));
        }

        let next_run_at = cron::compute_next_run_ms(
            &sched.trigger,
            sched.cron_expr.as_deref(),
            sched.interval_seconds,
            &sched.timezone,
            now,
        )?;

        let Some(updated) = self
            .store
            .0
            .record_execution_and_spend(&sched.schedule_id, estimated_cost_cents, now, next_run_at)
            .await?
        else {
            let _ = self.store.0.set_enabled(&sched.schedule_id, false).await;
            let _ = self
                .audit
                .record(
                    Some(&sched.owner_id),
                    "autocron.budget_exhausted",
                    "autocron_schedule",
                    Some(&sched.schedule_id),
                    "blocked",
                    json!({
                        "maxBudgetUsdCents": sched.max_budget_usd_cents,
                        "spentBudgetUsdCents": sched.spent_budget_usd_cents,
                        "requestedCostCents": estimated_cost_cents,
                    }),
                )
                .await;
            return Err(ServerError::Conflict(
                "autocron schedule budget cap reached; schedule disabled".into(),
            ));
        };

        let wf = self
            .workflows
            .get_by_id_internal(&updated.workflow_id)
            .await?;
        let run = self
            .workflows
            .execute_workflow_internal(&wf, boot_id, Some(&updated.schedule_id), input)
            .await?;

        let stream_session = format!("autocron-{}", updated.owner_id);
        let _ = emit_stream_event(
            &self.streams,
            &self.bus,
            &stream_session,
            Some(&run.run_id),
            StreamPayload::AutocronTriggered {
                schedule_id: updated.schedule_id.clone(),
                workflow_id: updated.workflow_id.clone(),
                triggered_at_ms: now,
            },
        )
        .await?;

        if updated.ambient_mode {
            let _ = self
                .audit
                .record(
                    Some(&updated.owner_id),
                    "autocron.ambient_notification",
                    "autocron_schedule",
                    Some(&updated.schedule_id),
                    "dispatched",
                    json!({
                        "workflowId": updated.workflow_id,
                        "runId": run.run_id,
                    }),
                )
                .await;
        }

        let remaining_budget_cents = updated
            .max_budget_usd_cents
            .map(|cap| cap.saturating_sub(updated.spent_budget_usd_cents));

        Ok(AutocronTickOutcome {
            schedule_id: updated.schedule_id,
            workflow_id: updated.workflow_id,
            run_id: run.run_id,
            triggered_at_ms: now,
            ambient_dispatched: updated.ambient_mode,
            remaining_budget_cents,
        })
    }
}
