//! Protected MCP host service built on `ironic::mcp::McpRouter` with actor-scoped BOLA
//! enforcement, tool allowlists, Valkey rate limits, and audit logging.

use std::{sync::Arc, time::Duration};

use agents_protocol::ApiKeyScope;
use ironic::{McpRouter, McpTool, prelude::*};
use serde_json::{Value, json};

use crate::{
    infra::{config::ServerConfig, error::ServerError},
    modules::{
        audit::services::AuditService,
        auth::{entities::AuthenticatedActor, services::StateStore},
        mcp::dto::{McpPolicyInfo, McpToolPolicy},
        sessions::{dto::ListSessionsQuery, services::SessionService},
        tasks::{dto::SubmitTaskDto, services::TaskService},
    },
};

tokio::task_local! {
    static CURRENT_MCP_ACTOR: AuthenticatedActor;
}

/// Executes `fut` with the authenticated caller bound for MCP tool handlers.
pub async fn with_mcp_actor<F, R>(actor: AuthenticatedActor, fut: F) -> R
where
    F: std::future::Future<Output = R>,
{
    CURRENT_MCP_ACTOR.scope(actor, fut).await
}

/// Orchestrates MCP tool registration and policy checks.
#[derive(Injectable)]
pub struct McpHostService {
    sessions: Arc<SessionService>,
    tasks: Arc<TaskService>,
    state: Arc<StateStore>,
    audit: Arc<AuditService>,
    config: Arc<ServerConfig>,
}

impl McpHostService {
    /// Returns the active MCP tool allowlist metadata.
    #[must_use]
    pub fn policy_info(&self) -> McpPolicyInfo {
        let policy = McpToolPolicy::default();
        let mut names: Vec<String> = policy.allowed_tools.keys().cloned().collect();
        names.sort();
        McpPolicyInfo {
            endpoint: "/mcp".to_owned(),
            allowed_tools: names,
        }
    }

    /// Verifies sliding-window Valkey rate limit for an MCP caller.
    ///
    /// # Errors
    ///
    /// Returns [`ServerError::RateLimited`] when exhausted.
    pub async fn check_rate_limit(&self, actor: &AuthenticatedActor) -> Result<(), ServerError> {
        let decision = self
            .state
            .0
            .check_rate_limit(
                &format!("mcp:{}", actor.user_id),
                self.config.rate_limit_max_requests,
                Duration::from_secs(self.config.rate_limit_window_secs),
            )
            .await?;
        if !decision.allowed {
            return Err(ServerError::RateLimited(
                "MCP tool rate limit exceeded".into(),
            ));
        }
        Ok(())
    }

    /// Validates that a `tools/call` invocation is allowlisted and permitted for `actor`.
    ///
    /// # Errors
    ///
    /// Returns [`ServerError::Forbidden`] if tool is unallowed or caller lacks scope.
    pub async fn authorize_tool_call(
        &self,
        actor: &AuthenticatedActor,
        tool_name: &str,
    ) -> Result<(), ServerError> {
        let policy = McpToolPolicy::default();
        let Some(required_scope) = policy.required_scope(tool_name) else {
            let _ = self
                .audit
                .record(
                    Some(&actor.user_id),
                    "mcp.tools_call",
                    "mcp_tool",
                    Some(tool_name),
                    "denied_allowlist",
                    json!({}),
                )
                .await;
            return Err(ServerError::Forbidden(format!(
                "MCP tool `{tool_name}` is not allowlisted"
            )));
        };
        if !actor.has_scope(required_scope) {
            let _ = self
                .audit
                .record(
                    Some(&actor.user_id),
                    "mcp.tools_call",
                    "mcp_tool",
                    Some(tool_name),
                    "denied_scope",
                    json!({ "required": required_scope.as_str() }),
                )
                .await;
            return Err(ServerError::Forbidden(format!(
                "MCP tool `{tool_name}` requires scope `{}`",
                required_scope.as_str()
            )));
        }
        let _ = self
            .audit
            .record(
                Some(&actor.user_id),
                "mcp.tools_call",
                "mcp_tool",
                Some(tool_name),
                "allowed",
                json!({}),
            )
            .await;
        Ok(())
    }

    /// Builds an [`ironic::McpRouter`] exposing allowlisted orchestrator tools.
    #[must_use]
    pub fn build_router(&self) -> McpRouter {
        let sessions = Arc::clone(&self.sessions);
        let tasks = Arc::clone(&self.tasks);

        let health_tool = McpTool::new(
            "health_summary",
            "Inspect orchestrator status and API generation",
            json!({ "type": "object", "properties": {} }),
            Arc::new(|_args| {
                Box::pin(async move {
                    Ok(json!({
                        "status": "ok",
                        "apiGeneration": agents_protocol::API_GENERATION,
                    }))
                })
            }),
        );

        let sessions_for_list = Arc::clone(&sessions);
        let list_sessions_tool = McpTool::new(
            "sessions_list",
            "List caller-owned agent sessions",
            json!({
                "type": "object",
                "properties": {
                    "limit": { "type": "integer", "minimum": 1, "maximum": 100 }
                }
            }),
            Arc::new(move |args| {
                let sessions = Arc::clone(&sessions_for_list);
                Box::pin(async move {
                    let actor = current_mcp_actor()?;
                    let limit = args
                        .get("limit")
                        .and_then(Value::as_u64)
                        .map(|n| n.clamp(1, 100) as u32);
                    let page = sessions
                        .list(
                            &actor,
                            &ListSessionsQuery {
                                cursor: None,
                                limit,
                                archived: None,
                            },
                        )
                        .await
                        .map_err(|e| e.to_string())?;
                    serde_json::to_value(page).map_err(|e| e.to_string())
                })
            }),
        );

        let sessions_for_get = Arc::clone(&sessions);
        let get_session_tool = McpTool::new(
            "session_get",
            "Fetch an owned session by ID",
            json!({
                "type": "object",
                "required": ["sessionId"],
                "properties": {
                    "sessionId": { "type": "string" }
                }
            }),
            Arc::new(move |args| {
                let sessions = Arc::clone(&sessions_for_get);
                Box::pin(async move {
                    let actor = current_mcp_actor()?;
                    let session_id = required_str(&args, "sessionId")?;
                    let session = sessions
                        .get_owned(&actor, session_id)
                        .await
                        .map_err(|e| e.to_string())?;
                    serde_json::to_value(session).map_err(|e| e.to_string())
                })
            }),
        );

        let tasks_for_submit = Arc::clone(&tasks);
        let submit_task_tool = McpTool::new(
            "task_submit",
            "Enqueue a sandbox task in a caller-owned session",
            json!({
                "type": "object",
                "required": ["sessionId", "prompt"],
                "properties": {
                    "sessionId": { "type": "string" },
                    "prompt": { "type": "string" },
                    "idempotencyKey": { "type": "string" }
                }
            }),
            Arc::new(move |args| {
                let tasks = Arc::clone(&tasks_for_submit);
                Box::pin(async move {
                    let actor = current_mcp_actor()?;
                    if !actor.has_scope(&ApiKeyScope::Write) {
                        return Err("write scope required".to_owned());
                    }
                    let session_id = required_str(&args, "sessionId")?;
                    let prompt = required_str(&args, "prompt")?.to_owned();
                    let idem = args
                        .get("idempotencyKey")
                        .and_then(Value::as_str)
                        .map(str::trim)
                        .filter(|s| !s.is_empty());
                    let task = tasks
                        .submit(
                            &actor,
                            session_id,
                            idem,
                            &SubmitTaskDto {
                                prompt,
                                timeout_seconds: None,
                                mode: None,
                                sandbox: None,
                                exec: None,
                            },
                        )
                        .await
                        .map_err(|e| e.to_string())?;
                    serde_json::to_value(task).map_err(|e| e.to_string())
                })
            }),
        );

        let tasks_for_status = Arc::clone(&tasks);
        let task_status_tool = McpTool::new(
            "task_status",
            "Inspect status of an owned task",
            json!({
                "type": "object",
                "required": ["taskId"],
                "properties": {
                    "taskId": { "type": "string" }
                }
            }),
            Arc::new(move |args| {
                let tasks = Arc::clone(&tasks_for_status);
                Box::pin(async move {
                    let actor = current_mcp_actor()?;
                    let task_id = required_str(&args, "taskId")?;
                    let task = tasks
                        .get_owned(&actor, task_id)
                        .await
                        .map_err(|e| e.to_string())?;
                    serde_json::to_value(task).map_err(|e| e.to_string())
                })
            }),
        );

        McpRouter::new()
            .with_name("agents-server-mcp")
            .with_version(env!("CARGO_PKG_VERSION"))
            .register_tools(vec![
                health_tool,
                list_sessions_tool,
                get_session_tool,
                submit_task_tool,
                task_status_tool,
            ])
    }
}

fn current_mcp_actor() -> Result<AuthenticatedActor, String> {
    CURRENT_MCP_ACTOR
        .try_with(Clone::clone)
        .map_err(|_| "authenticated MCP actor unavailable".to_owned())
}

fn required_str<'a>(val: &'a Value, field: &str) -> Result<&'a str, String> {
    val.get(field)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| format!("missing string parameter `{field}`"))
}
