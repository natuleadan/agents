//! Automated DAG workflow and Autocron scheduler protocol contracts.

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json::Value;

/// Trigger source initiating an automated workflow execution.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TriggerKind {
    Cron,
    IntervalSeconds,
    Webhook,
    Manual,
    Unknown(String),
}

impl TriggerKind {
    pub fn as_str(&self) -> &str {
        match self {
            Self::Cron => "cron",
            Self::IntervalSeconds => "interval_seconds",
            Self::Webhook => "webhook",
            Self::Manual => "manual",
            Self::Unknown(raw) => raw.as_str(),
        }
    }
}

impl Serialize for TriggerKind {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for TriggerKind {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let raw = String::deserialize(deserializer)?;
        Ok(match raw.as_str() {
            "cron" => Self::Cron,
            "interval_seconds" => Self::IntervalSeconds,
            "webhook" => Self::Webhook,
            "manual" => Self::Manual,
            _ => Self::Unknown(raw),
        })
    }
}

/// Node inside an automated workflow DAG.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowNode {
    pub node_id: String,
    pub kind: String,
    pub config: Value,
}

/// Directed dependency edge connecting two workflow nodes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowEdge {
    pub from_node_id: String,
    pub to_node_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub condition: Option<String>,
}

/// Serializable workflow DAG definition persisted in PostgreSQL.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowDag {
    pub workflow_id: String,
    pub name: String,
    pub nodes: Vec<WorkflowNode>,
    pub edges: Vec<WorkflowEdge>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub webhook_slug: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub render_slug: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub approved: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created_by_agent: Option<bool>,
}

/// Persistent Autocron schedule specification with timezone and budget guards.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AutocronSchedule {
    pub schedule_id: String,
    pub workflow_id: String,
    pub trigger: TriggerKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cron_expr: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub interval_seconds: Option<u64>,
    pub timezone: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_budget_usd_cents: Option<u64>,
    #[serde(default)]
    pub ambient_mode: bool,
    pub enabled: bool,
}
