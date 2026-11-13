//! MCP host tool policy configuration and JSON-RPC inspection DTOs.

use std::collections::HashMap;

use agents_protocol::ApiKeyScope;
use ironic::OpenApiSchema;
use serde::{Deserialize, Serialize};

/// Per-tool allowlist and required permission scope enforced before executing MCP calls.
#[derive(Debug, Clone)]
pub struct McpToolPolicy {
    pub allowed_tools: HashMap<String, ApiKeyScope>,
}

impl Default for McpToolPolicy {
    fn default() -> Self {
        let mut allowed_tools = HashMap::new();
        allowed_tools.insert("health_summary".to_owned(), ApiKeyScope::Read);
        allowed_tools.insert("sessions_list".to_owned(), ApiKeyScope::Read);
        allowed_tools.insert("session_get".to_owned(), ApiKeyScope::Read);
        allowed_tools.insert("task_submit".to_owned(), ApiKeyScope::Write);
        allowed_tools.insert("task_status".to_owned(), ApiKeyScope::Read);
        Self { allowed_tools }
    }
}

impl McpToolPolicy {
    #[must_use]
    pub fn required_scope(&self, tool_name: &str) -> Option<&ApiKeyScope> {
        self.allowed_tools.get(tool_name)
    }
}

/// Summary of the MCP host policy returned on admin/metadata inspection.
#[derive(Debug, Clone, Serialize, Deserialize, OpenApiSchema)]
#[serde(rename_all = "camelCase")]
pub struct McpPolicyInfo {
    pub endpoint: String,
    pub allowed_tools: Vec<String>,
}
