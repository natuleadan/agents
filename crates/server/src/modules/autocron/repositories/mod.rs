//! Persistence repository for Autocron schedules and atomic budget accounting.

use std::{collections::HashMap, sync::Arc};

use agents_protocol::TriggerKind;
use sqlx::{PgPool, Row};
use tokio::sync::Mutex;

use crate::{
    infra::{
        error::ServerError,
        state::{BoxFuture, now_ms},
    },
    modules::autocron::entities::AutocronScheduleEntity,
};

/// Concrete DI wrapper around [`AutocronRepository`].
#[derive(Clone)]
pub struct AutocronStore(pub Arc<dyn AutocronRepository>);

impl Default for AutocronStore {
    fn default() -> Self {
        Self(InMemoryAutocronRepository::new())
    }
}

/// Persistence contract for Autocron schedules and atomic budget enforcement.
pub trait AutocronRepository: Send + Sync + 'static {
    fn create_schedule<'a>(
        &'a self,
        sched: AutocronScheduleEntity,
    ) -> BoxFuture<'a, Result<AutocronScheduleEntity, ServerError>>;

    fn find_schedule<'a>(
        &'a self,
        schedule_id: &'a str,
    ) -> BoxFuture<'a, Result<Option<AutocronScheduleEntity>, ServerError>>;

    fn list_schedules_for_owner<'a>(
        &'a self,
        owner_id: &'a str,
    ) -> BoxFuture<'a, Result<Vec<AutocronScheduleEntity>, ServerError>>;

    fn set_enabled<'a>(
        &'a self,
        schedule_id: &'a str,
        enabled: bool,
    ) -> BoxFuture<'a, Result<Option<AutocronScheduleEntity>, ServerError>>;

    fn list_due_schedules<'a>(
        &'a self,
        now_ms: u64,
    ) -> BoxFuture<'a, Result<Vec<AutocronScheduleEntity>, ServerError>>;

    /// Atomically checks budget ceiling and records spend + next_run_at.
    /// Returns `Ok(None)` when budget cap would be exceeded or schedule is disabled.
    fn record_execution_and_spend<'a>(
        &'a self,
        schedule_id: &'a str,
        additional_cost_cents: u64,
        executed_at_ms: u64,
        next_run_at: Option<u64>,
    ) -> BoxFuture<'a, Result<Option<AutocronScheduleEntity>, ServerError>>;
}

/// PostgreSQL repository for Autocron schedules.
#[derive(Clone)]
pub struct PgAutocronRepository {
    pool: PgPool,
}

impl PgAutocronRepository {
    #[must_use]
    pub fn new(pool: PgPool) -> Arc<Self> {
        Arc::new(Self { pool })
    }
}

impl AutocronRepository for PgAutocronRepository {
    fn create_schedule<'a>(
        &'a self,
        sched: AutocronScheduleEntity,
    ) -> BoxFuture<'a, Result<AutocronScheduleEntity, ServerError>> {
        Box::pin(async move {
            sqlx::query(
                "INSERT INTO autocron_schedules (
                    schedule_id, owner_id, workflow_id, trigger_kind, cron_expr,
                    interval_seconds, timezone, max_budget_usd_cents, spent_budget_usd_cents,
                    ambient_mode, enabled, next_run_at, last_run_at, created_at, updated_at
                 ) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15)",
            )
            .bind(&sched.schedule_id)
            .bind(&sched.owner_id)
            .bind(&sched.workflow_id)
            .bind(sched.trigger.as_str())
            .bind(&sched.cron_expr)
            .bind(sched.interval_seconds.map(|v| v as i64))
            .bind(&sched.timezone)
            .bind(sched.max_budget_usd_cents.map(|v| v as i64))
            .bind(sched.spent_budget_usd_cents as i64)
            .bind(sched.ambient_mode)
            .bind(sched.enabled)
            .bind(sched.next_run_at.map(|v| v as i64))
            .bind(sched.last_run_at.map(|v| v as i64))
            .bind(sched.created_at as i64)
            .bind(sched.updated_at as i64)
            .execute(&self.pool)
            .await
            .map_err(|e| ServerError::Internal(format!("create schedule failed: {e}")))?;
            Ok(sched)
        })
    }

    fn find_schedule<'a>(
        &'a self,
        schedule_id: &'a str,
    ) -> BoxFuture<'a, Result<Option<AutocronScheduleEntity>, ServerError>> {
        Box::pin(async move {
            let row = sqlx::query(
                "SELECT schedule_id, owner_id, workflow_id, trigger_kind, cron_expr,
                        interval_seconds, timezone, max_budget_usd_cents, spent_budget_usd_cents,
                        ambient_mode, enabled, next_run_at, last_run_at, created_at, updated_at
                 FROM autocron_schedules WHERE schedule_id = $1",
            )
            .bind(schedule_id)
            .fetch_optional(&self.pool)
            .await
            .map_err(|e| ServerError::Internal(format!("find schedule failed: {e}")))?;
            row.map(row_to_schedule).transpose()
        })
    }

    fn list_schedules_for_owner<'a>(
        &'a self,
        owner_id: &'a str,
    ) -> BoxFuture<'a, Result<Vec<AutocronScheduleEntity>, ServerError>> {
        Box::pin(async move {
            let rows = sqlx::query(
                "SELECT schedule_id, owner_id, workflow_id, trigger_kind, cron_expr,
                        interval_seconds, timezone, max_budget_usd_cents, spent_budget_usd_cents,
                        ambient_mode, enabled, next_run_at, last_run_at, created_at, updated_at
                 FROM autocron_schedules WHERE owner_id = $1 ORDER BY created_at DESC",
            )
            .bind(owner_id)
            .fetch_all(&self.pool)
            .await
            .map_err(|e| ServerError::Internal(format!("list schedules failed: {e}")))?;
            rows.into_iter().map(row_to_schedule).collect()
        })
    }

    fn set_enabled<'a>(
        &'a self,
        schedule_id: &'a str,
        enabled: bool,
    ) -> BoxFuture<'a, Result<Option<AutocronScheduleEntity>, ServerError>> {
        Box::pin(async move {
            let row = sqlx::query(
                "UPDATE autocron_schedules
                 SET enabled = $2, updated_at = $3
                 WHERE schedule_id = $1
                 RETURNING schedule_id, owner_id, workflow_id, trigger_kind, cron_expr,
                           interval_seconds, timezone, max_budget_usd_cents, spent_budget_usd_cents,
                           ambient_mode, enabled, next_run_at, last_run_at, created_at, updated_at",
            )
            .bind(schedule_id)
            .bind(enabled)
            .bind(now_ms() as i64)
            .fetch_optional(&self.pool)
            .await
            .map_err(|e| ServerError::Internal(format!("set_enabled failed: {e}")))?;
            row.map(row_to_schedule).transpose()
        })
    }

    fn list_due_schedules<'a>(
        &'a self,
        current_ms: u64,
    ) -> BoxFuture<'a, Result<Vec<AutocronScheduleEntity>, ServerError>> {
        Box::pin(async move {
            let rows = sqlx::query(
                "SELECT schedule_id, owner_id, workflow_id, trigger_kind, cron_expr,
                        interval_seconds, timezone, max_budget_usd_cents, spent_budget_usd_cents,
                        ambient_mode, enabled, next_run_at, last_run_at, created_at, updated_at
                 FROM autocron_schedules
                 WHERE enabled = TRUE AND next_run_at IS NOT NULL AND next_run_at <= $1",
            )
            .bind(current_ms as i64)
            .fetch_all(&self.pool)
            .await
            .map_err(|e| ServerError::Internal(format!("list_due_schedules failed: {e}")))?;
            rows.into_iter().map(row_to_schedule).collect()
        })
    }

    fn record_execution_and_spend<'a>(
        &'a self,
        schedule_id: &'a str,
        additional_cost_cents: u64,
        executed_at_ms: u64,
        next_run_at: Option<u64>,
    ) -> BoxFuture<'a, Result<Option<AutocronScheduleEntity>, ServerError>> {
        Box::pin(async move {
            let row = sqlx::query(
                "UPDATE autocron_schedules
                 SET spent_budget_usd_cents = spent_budget_usd_cents + $2,
                     last_run_at = $3,
                     next_run_at = $4,
                     updated_at = $3
                 WHERE schedule_id = $1
                   AND enabled = TRUE
                   AND (max_budget_usd_cents IS NULL OR spent_budget_usd_cents + $2 <= max_budget_usd_cents)
                 RETURNING schedule_id, owner_id, workflow_id, trigger_kind, cron_expr,
                           interval_seconds, timezone, max_budget_usd_cents, spent_budget_usd_cents,
                           ambient_mode, enabled, next_run_at, last_run_at, created_at, updated_at",
            )
            .bind(schedule_id)
            .bind(additional_cost_cents as i64)
            .bind(executed_at_ms as i64)
            .bind(next_run_at.map(|v| v as i64))
            .fetch_optional(&self.pool)
            .await
            .map_err(|e| ServerError::Internal(format!("record spend failed: {e}")))?;
            row.map(row_to_schedule).transpose()
        })
    }
}

fn row_to_schedule(row: sqlx::postgres::PgRow) -> Result<AutocronScheduleEntity, ServerError> {
    let raw_trigger: String = row.get("trigger_kind");
    let trigger = match raw_trigger.as_str() {
        "cron" => TriggerKind::Cron,
        "interval_seconds" => TriggerKind::IntervalSeconds,
        "webhook" => TriggerKind::Webhook,
        "manual" => TriggerKind::Manual,
        other => TriggerKind::Unknown(other.to_owned()),
    };
    Ok(AutocronScheduleEntity {
        schedule_id: row.get("schedule_id"),
        owner_id: row.get("owner_id"),
        workflow_id: row.get("workflow_id"),
        trigger,
        cron_expr: row.get("cron_expr"),
        interval_seconds: row
            .get::<Option<i64>, _>("interval_seconds")
            .map(|v| v.max(0) as u64),
        timezone: row.get("timezone"),
        max_budget_usd_cents: row
            .get::<Option<i64>, _>("max_budget_usd_cents")
            .map(|v| v.max(0) as u64),
        spent_budget_usd_cents: row.get::<i64, _>("spent_budget_usd_cents").max(0) as u64,
        ambient_mode: row.get("ambient_mode"),
        enabled: row.get("enabled"),
        next_run_at: row
            .get::<Option<i64>, _>("next_run_at")
            .map(|v| v.max(0) as u64),
        last_run_at: row
            .get::<Option<i64>, _>("last_run_at")
            .map(|v| v.max(0) as u64),
        created_at: row.get::<i64, _>("created_at").max(0) as u64,
        updated_at: row.get::<i64, _>("updated_at").max(0) as u64,
    })
}

/// Deterministic in-memory Autocron schedule repository.
#[derive(Clone, Default)]
pub struct InMemoryAutocronRepository {
    schedules: Arc<Mutex<HashMap<String, AutocronScheduleEntity>>>,
}

impl InMemoryAutocronRepository {
    #[must_use]
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }
}

impl AutocronRepository for InMemoryAutocronRepository {
    fn create_schedule<'a>(
        &'a self,
        sched: AutocronScheduleEntity,
    ) -> BoxFuture<'a, Result<AutocronScheduleEntity, ServerError>> {
        Box::pin(async move {
            self.schedules
                .lock()
                .await
                .insert(sched.schedule_id.clone(), sched.clone());
            Ok(sched)
        })
    }

    fn find_schedule<'a>(
        &'a self,
        schedule_id: &'a str,
    ) -> BoxFuture<'a, Result<Option<AutocronScheduleEntity>, ServerError>> {
        Box::pin(async move { Ok(self.schedules.lock().await.get(schedule_id).cloned()) })
    }

    fn list_schedules_for_owner<'a>(
        &'a self,
        owner_id: &'a str,
    ) -> BoxFuture<'a, Result<Vec<AutocronScheduleEntity>, ServerError>> {
        Box::pin(async move {
            Ok(self
                .schedules
                .lock()
                .await
                .values()
                .filter(|s| s.owner_id == owner_id)
                .cloned()
                .collect())
        })
    }

    fn set_enabled<'a>(
        &'a self,
        schedule_id: &'a str,
        enabled: bool,
    ) -> BoxFuture<'a, Result<Option<AutocronScheduleEntity>, ServerError>> {
        Box::pin(async move {
            let mut guard = self.schedules.lock().await;
            let Some(sched) = guard.get_mut(schedule_id) else {
                return Ok(None);
            };
            sched.enabled = enabled;
            sched.updated_at = now_ms();
            Ok(Some(sched.clone()))
        })
    }

    fn list_due_schedules<'a>(
        &'a self,
        current_ms: u64,
    ) -> BoxFuture<'a, Result<Vec<AutocronScheduleEntity>, ServerError>> {
        Box::pin(async move {
            Ok(self
                .schedules
                .lock()
                .await
                .values()
                .filter(|s| s.enabled && s.next_run_at.is_some_and(|next| next <= current_ms))
                .cloned()
                .collect())
        })
    }

    fn record_execution_and_spend<'a>(
        &'a self,
        schedule_id: &'a str,
        additional_cost_cents: u64,
        executed_at_ms: u64,
        next_run_at: Option<u64>,
    ) -> BoxFuture<'a, Result<Option<AutocronScheduleEntity>, ServerError>> {
        Box::pin(async move {
            let mut guard = self.schedules.lock().await;
            let Some(sched) = guard.get_mut(schedule_id) else {
                return Ok(None);
            };
            if !sched.enabled {
                return Ok(None);
            }
            let next_spent = sched
                .spent_budget_usd_cents
                .saturating_add(additional_cost_cents);
            if sched
                .max_budget_usd_cents
                .is_some_and(|cap| next_spent > cap)
            {
                return Ok(None);
            }
            sched.spent_budget_usd_cents = next_spent;
            sched.last_run_at = Some(executed_at_ms);
            sched.next_run_at = next_run_at;
            sched.updated_at = executed_at_ms;
            Ok(Some(sched.clone()))
        })
    }
}
