//! Domain entities for Autocron schedules, execution outcomes, and boot recovery reports.

use agents_protocol::TriggerKind;
use serde::{Deserialize, Serialize};

/// Persistent Autocron schedule record backed by `autocron_schedules`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AutocronScheduleEntity {
    pub schedule_id: String,
    pub owner_id: String,
    pub workflow_id: String,
    pub trigger: TriggerKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cron_expr: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub interval_seconds: Option<u64>,
    pub timezone: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_budget_usd_cents: Option<u64>,
    pub spent_budget_usd_cents: u64,
    pub ambient_mode: bool,
    pub enabled: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_run_at: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_run_at: Option<u64>,
    pub created_at: u64,
    pub updated_at: u64,
}

/// Summary of an individual Autocron execution dispatch.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AutocronTickOutcome {
    pub schedule_id: String,
    pub workflow_id: String,
    pub run_id: String,
    pub triggered_at_ms: u64,
    pub ambient_dispatched: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub remaining_budget_cents: Option<u64>,
}

/// Startup reconciliation report across interrupted workflow runs and due cron schedules.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AutocronRecoveryReport {
    pub boot_id: String,
    pub resumed_runs: u32,
    pub triggered_due_schedules: u32,
}
