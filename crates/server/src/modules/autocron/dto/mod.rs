//! Validated HTTP DTOs for Autocron scheduling, execution, toggling, and webhook triggers.

use agents_protocol::TriggerKind;
use garde::Validate;
use ironic::OpenApiSchema;
use serde::Deserialize;
use serde_json::Value;

/// Request payload creating an Autocron schedule bound to a workflow DAG.
#[derive(Debug, Clone, Deserialize, Validate, OpenApiSchema)]
#[serde(rename_all = "camelCase")]
pub struct CreateScheduleDto {
    #[garde(length(min = 1, max = 128))]
    pub workflow_id: String,
    #[garde(skip)]
    pub trigger: TriggerKind,
    #[garde(skip)]
    pub cron_expr: Option<String>,
    #[garde(skip)]
    pub interval_seconds: Option<u64>,
    #[garde(skip)]
    pub timezone: Option<String>,
    #[garde(skip)]
    pub max_budget_usd_cents: Option<u64>,
    #[garde(skip)]
    pub ambient_mode: Option<bool>,
    #[garde(skip)]
    pub enabled: Option<bool>,
}

/// Request payload triggering a scheduled run manually with optional estimated cost.
#[derive(Debug, Clone, Default, Deserialize, Validate)]
#[serde(rename_all = "camelCase")]
pub struct ExecuteScheduleDto {
    #[garde(skip)]
    pub estimated_cost_cents: Option<u64>,
    #[garde(skip)]
    pub input: Option<Value>,
    #[garde(skip)]
    pub boot_id: Option<String>,
}

impl OpenApiSchema for ExecuteScheduleDto {
    fn openapi_schema() -> Value {
        serde_json::json!({
            "type": "object",
            "title": "ExecuteScheduleDto",
            "properties": {
                "estimatedCostCents": { "type": "integer", "nullable": true, "example": 15 },
                "input": { "type": "object", "nullable": true, "example": { "key": "value" } },
                "bootId": { "type": "string", "nullable": true, "example": "boot_01j7xyz" }
            }
        })
    }
}

/// Request payload enabling or disabling an existing schedule.
#[derive(Debug, Clone, Deserialize, Validate, OpenApiSchema)]
#[serde(rename_all = "camelCase")]
pub struct ToggleScheduleDto {
    #[garde(skip)]
    pub enabled: bool,
}

/// Request payload triggering a webhook-backed Autocron schedule.
#[derive(Debug, Clone, Default, Deserialize, Validate)]
#[serde(rename_all = "camelCase")]
pub struct TriggerWebhookDto {
    #[garde(skip)]
    pub payload: Option<Value>,
    #[garde(skip)]
    pub estimated_cost_cents: Option<u64>,
}

impl OpenApiSchema for TriggerWebhookDto {
    fn openapi_schema() -> Value {
        serde_json::json!({
            "type": "object",
            "title": "TriggerWebhookDto",
            "properties": {
                "payload": { "type": "object", "nullable": true, "example": { "event": "order.created" } },
                "estimatedCostCents": { "type": "integer", "nullable": true, "example": 5 }
            }
        })
    }
}
