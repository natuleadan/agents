//! Domain entities for Autocron schedules, execution outcomes, and boot recovery reports.

use agents_protocol::TriggerKind;
use ironic::OpenApiSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

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

impl OpenApiSchema for AutocronScheduleEntity {
    fn openapi_schema() -> Value {
        serde_json::json!({
            "type": "object",
            "title": "AutocronScheduleEntity",
            "required": ["scheduleId", "ownerId", "workflowId", "trigger", "timezone", "spentBudgetUsdCents", "ambientMode", "enabled", "createdAt", "updatedAt"],
            "properties": {
                "scheduleId": { "type": "string", "example": "sch_01j7xyz" },
                "ownerId": { "type": "string", "example": "usr_01j7abc" },
                "workflowId": { "type": "string", "example": "wf_01j7dag001" },
                "trigger": TriggerKind::openapi_schema(),
                "cronExpr": { "type": "string", "nullable": true, "example": "0 */5 * * * *" },
                "intervalSeconds": { "type": "integer", "nullable": true, "example": 300 },
                "timezone": { "type": "string", "example": "UTC" },
                "maxBudgetUsdCents": { "type": "integer", "nullable": true, "example": 1000 },
                "spentBudgetUsdCents": { "type": "integer", "example": 120 },
                "ambientMode": { "type": "boolean", "example": false },
                "enabled": { "type": "boolean", "example": true },
                "nextRunAt": { "type": "integer", "nullable": true, "example": 1731513900 },
                "lastRunAt": { "type": "integer", "nullable": true, "example": 1731513600 },
                "createdAt": { "type": "integer", "example": 1731513000 },
                "updatedAt": { "type": "integer", "example": 1731513600 }
            }
        })
    }
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

impl OpenApiSchema for AutocronTickOutcome {
    fn openapi_schema() -> Value {
        serde_json::json!({
            "type": "object",
            "title": "AutocronTickOutcome",
            "required": ["scheduleId", "workflowId", "runId", "triggeredAtMs", "ambientDispatched"],
            "properties": {
                "scheduleId": { "type": "string", "example": "sch_01j7xyz" },
                "workflowId": { "type": "string", "example": "wf_01j7dag001" },
                "runId": { "type": "string", "example": "run_01j7run001" },
                "triggeredAtMs": { "type": "integer", "example": 1731513600000_u64 },
                "ambientDispatched": { "type": "boolean", "example": false },
                "remainingBudgetCents": { "type": "integer", "nullable": true, "example": 880 }
            }
        })
    }
}

/// Startup reconciliation report across interrupted workflow runs and due cron schedules.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, OpenApiSchema)]
#[serde(rename_all = "camelCase")]
pub struct AutocronRecoveryReport {
    pub boot_id: String,
    pub resumed_runs: u32,
    pub triggered_due_schedules: u32,
}
