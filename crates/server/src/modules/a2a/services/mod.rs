//! A2A protocol service built natively on `a2a-lf` types, `IdentityService`, and orchestrator eventing.

use std::{collections::HashMap, sync::Arc};

use a2a::{
    AgentCapabilities, AgentCard, AgentExtension, AgentInterface, AgentProvider, AgentSkill,
    ApiKeySecurityScheme, Artifact, JsonRpcError, JsonRpcId, JsonRpcRequest, JsonRpcResponse,
    Message, Part, Role, SecurityScheme, Task, TaskState, TaskStatus,
};
use agents_protocol::{
    HEADER_AGENT_IDENTITY_CODE, HEADER_AGENT_INSTALLATION_ID, HEADER_AGENT_ORGANIZATION,
};
use axum::http::HeaderMap;
use chrono::Utc;
use garde::Validate;
use ironic::{Injectable, OpenApiSchema};
use reqwest::Url;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tracing::info;

use crate::{
    infra::{error::ServerError, events::AppEventBus},
    modules::identity::services::{IdentityService, OutboundInteractionRecord},
};

use super::repositories::A2aTaskStore;

const JSONRPC_INVALID_PARAMS: i32 = -32602;
const JSONRPC_METHOD_NOT_FOUND: i32 = -32601;
const JSONRPC_TASK_NOT_FOUND: i32 = -32001;

/// Request payload for dispatching an outbound A2A JSON-RPC message to a remote A2A agent.
#[derive(Clone, Debug, Deserialize, Validate, OpenApiSchema)]
#[serde(rename_all = "camelCase")]
pub struct OutboundA2aSendDto {
    #[garde(length(min = 8, max = 512))]
    pub target_url: String,
    #[garde(length(min = 1, max = 16384))]
    pub text: String,
    #[garde(length(max = 128))]
    pub context_id: Option<String>,
}

/// Response returned after sending an outbound A2A task/message.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OutboundA2aResponse {
    pub target_url: String,
    pub status_code: u16,
    pub injected_org: String,
    pub injected_installation_id: String,
    pub response: Value,
}

impl OpenApiSchema for OutboundA2aResponse {
    fn openapi_schema() -> Value {
        json!({
            "type": "object",
            "properties": {
                "targetUrl": { "type": "string" },
                "statusCode": { "type": "integer" },
                "injectedOrg": { "type": "string" },
                "injectedInstallationId": { "type": "string" },
                "response": { "type": "object" }
            },
            "required": ["targetUrl", "statusCode", "injectedOrg", "injectedInstallationId", "response"]
        })
    }
}

/// Orchestrates A2A discovery (`AgentCard`), inbound JSON-RPC postal gateway, and outbound A2A calls.
#[derive(Clone, Injectable)]
pub struct A2aService {
    store: Arc<A2aTaskStore>,
    identity: Arc<IdentityService>,
    events: Arc<AppEventBus>,
}

impl A2aService {
    #[must_use]
    pub fn new(
        store: Arc<A2aTaskStore>,
        identity: Arc<IdentityService>,
        events: Arc<AppEventBus>,
    ) -> Self {
        Self {
            store,
            identity,
            events,
        }
    }

    /// Builds the official A2A v1 `AgentCard` reflecting the active installation organization.
    pub async fn build_agent_card(&self, base_url: &str) -> AgentCard {
        let active_identity = self
            .identity
            .resolve_installation_identity()
            .await
            .ok()
            .flatten();
        let org_name = active_identity
            .as_ref()
            .map(|i| i.organization_name.clone())
            .unwrap_or_else(|| "Agents Orchestrator".to_owned());
        let installation_id = active_identity
            .as_ref()
            .map(|i| i.installation_id.clone())
            .unwrap_or_else(|| "unconfigured".to_owned());

        let base = base_url.trim_end_matches('/');
        let mut ext_params = HashMap::new();
        ext_params.insert(
            "organizationHeader".to_owned(),
            json!(HEADER_AGENT_ORGANIZATION),
        );
        ext_params.insert(
            "installationIdHeader".to_owned(),
            json!(HEADER_AGENT_INSTALLATION_ID),
        );
        ext_params.insert(
            "identityCodeHeader".to_owned(),
            json!(HEADER_AGENT_IDENTITY_CODE),
        );
        ext_params.insert("installationId".to_owned(), json!(installation_id));

        let mut security_schemes = HashMap::new();
        security_schemes.insert(
            "agentOrgHeader".to_owned(),
            SecurityScheme::ApiKey(ApiKeySecurityScheme {
                description: Some("Organization identifier header".to_owned()),
                location: "header".to_owned(),
                name: HEADER_AGENT_ORGANIZATION.to_owned(),
            }),
        );
        security_schemes.insert(
            "agentInstallationHeader".to_owned(),
            SecurityScheme::ApiKey(ApiKeySecurityScheme {
                description: Some("Alphanumeric installation identifier header".to_owned()),
                location: "header".to_owned(),
                name: HEADER_AGENT_INSTALLATION_ID.to_owned(),
            }),
        );

        AgentCard {
            name: format!("{org_name} Orchestrator"),
            description: "Zero-trust multi-agent orchestrator acting as a postal A2A gateway for sandboxed workers.".to_owned(),
            version: env!("CARGO_PKG_VERSION").to_owned(),
            supported_interfaces: vec![AgentInterface {
                url: format!("{base}/a2a"),
                protocol_binding: "JSONRPC".to_owned(),
                tenant: None,
                protocol_version: "1.0".to_owned(),
            }],
            capabilities: AgentCapabilities {
                streaming: Some(false),
                push_notifications: Some(false),
                extensions: Some(vec![AgentExtension {
                    uri: "urn:agents:installation-identity:v1".to_owned(),
                    description: Some(
                        "Installation-wide organization, alphanumeric ID, and signed identity code headers."
                            .to_owned(),
                    ),
                    required: Some(false),
                    params: Some(ext_params),
                }]),
                extended_agent_card: Some(false),
            },
            default_input_modes: vec!["text/plain".to_owned(), "application/json".to_owned()],
            default_output_modes: vec!["text/plain".to_owned(), "application/json".to_owned()],
            skills: vec![
                AgentSkill {
                    id: "orchestrator.message".to_owned(),
                    name: "Orchestrator Postal Intake".to_owned(),
                    description: "Receives external A2A messages, records them in the permanent ledger, and dispatches work internally without exposing sandboxes.".to_owned(),
                    tags: vec!["a2a".to_owned(), "orchestrator".to_owned(), "sandbox".to_owned()],
                    examples: Some(vec!["Analyze repository status and return summary".to_owned()]),
                    input_modes: Some(vec!["text/plain".to_owned()]),
                    output_modes: Some(vec!["application/json".to_owned(), "text/plain".to_owned()]),
                    security_requirements: None,
                },
            ],
            provider: Some(AgentProvider {
                organization: org_name,
                url: base.to_owned(),
            }),
            documentation_url: Some(format!("{base}/docs")),
            icon_url: None,
            security_schemes: Some(security_schemes),
            security_requirements: None,
            signatures: None,
        }
    }

    /// Processes an inbound A2A JSON-RPC 2.0 request and records it in the permanent ledger.
    pub async fn handle_jsonrpc(
        &self,
        req: JsonRpcRequest,
        headers: &HeaderMap,
    ) -> JsonRpcResponse {
        let id = req.id.clone();
        let method = req.method.as_str();

        let inbound_entry = match self
            .identity
            .record_inbound(
                "a2a",
                "/a2a",
                "POST",
                headers,
                None,
                json!({ "rpcMethod": method }),
            )
            .await
        {
            Ok(entry) => entry,
            Err(err) => {
                return JsonRpcResponse::error(
                    id,
                    JsonRpcError {
                        code: -32001,
                        message: err.to_string(),
                        data: None,
                    },
                );
            }
        };

        match method {
            "message/send" | "SendMessage" => {
                self.handle_message_send(
                    id,
                    req.params,
                    inbound_entry.remote_org,
                    inbound_entry.remote_installation_id,
                )
                .await
            }
            "tasks/get" | "GetTask" => self.handle_tasks_get(id, req.params).await,
            "tasks/cancel" | "CancelTask" => self.handle_tasks_cancel(id, req.params).await,
            _ => JsonRpcResponse::error(
                id,
                JsonRpcError {
                    code: JSONRPC_METHOD_NOT_FOUND,
                    message: format!("unsupported A2A method `{method}`"),
                    data: None,
                },
            ),
        }
    }

    async fn handle_message_send(
        &self,
        id: JsonRpcId,
        params: Option<Value>,
        caller_org: Option<String>,
        caller_installation_id: Option<String>,
    ) -> JsonRpcResponse {
        let Some(params_val) = params else {
            return invalid_params(id, "missing params for message/send");
        };

        let msg_val = params_val.get("message").cloned().unwrap_or(params_val);
        let inbound_msg: Message = match serde_json::from_value(msg_val) {
            Ok(m) => m,
            Err(err) => return invalid_params(id, &format!("invalid A2A message payload: {err}")),
        };

        let task_id = inbound_msg
            .task_id
            .clone()
            .unwrap_or_else(|| format!("a2a-{}", uuid::Uuid::new_v4()));
        let context_id = inbound_msg
            .context_id
            .clone()
            .unwrap_or_else(|| format!("ctx-{}", uuid::Uuid::new_v4()));
        let text_preview = inbound_msg.text().unwrap_or("").to_owned();

        let reply_text = format!(
            "Accepted by orchestrator postal gateway (task={task_id}, context={context_id}, caller_org={})",
            caller_org.as_deref().unwrap_or("anonymous")
        );
        let reply_msg = Message {
            message_id: uuid::Uuid::new_v4().to_string(),
            context_id: Some(context_id.clone()),
            task_id: Some(task_id.clone()),
            role: Role::Agent,
            parts: vec![Part::text(reply_text.clone())],
            metadata: None,
            extensions: None,
            reference_task_ids: None,
        };

        let task = Task {
            id: task_id.clone(),
            context_id,
            status: TaskStatus {
                state: TaskState::Completed,
                message: Some(reply_msg),
                timestamp: Some(Utc::now()),
            },
            artifacts: Some(vec![Artifact {
                artifact_id: uuid::Uuid::new_v4().to_string(),
                name: Some("postal-receipt".to_owned()),
                description: Some("A2A postal intake receipt".to_owned()),
                parts: vec![Part::data(json!({
                    "taskId": task_id,
                    "callerOrg": caller_org,
                    "callerInstallationId": caller_installation_id,
                    "receivedChars": text_preview.len(),
                }))],
                metadata: None,
                extensions: None,
            }]),
            history: Some(vec![inbound_msg]),
            metadata: None,
        };

        match self
            .store
            .0
            .save_task(task, caller_org, caller_installation_id)
            .await
        {
            Ok(saved) => {
                let _ = &self.events;
                info!(target: "agents.a2a.intake", task_id = %saved.id, "accepted inbound A2A task");
                let result = serde_json::to_value(&saved).unwrap_or_else(|_| json!({}));
                JsonRpcResponse::success(id, result)
            }
            Err(err) => JsonRpcResponse::error(
                id,
                JsonRpcError {
                    code: -32603,
                    message: err.to_string(),
                    data: None,
                },
            ),
        }
    }

    async fn handle_tasks_get(&self, id: JsonRpcId, params: Option<Value>) -> JsonRpcResponse {
        let Some(task_id) = extract_task_id(params.as_ref()) else {
            return invalid_params(id, "missing task id");
        };
        match self.store.0.get_task(&task_id).await {
            Ok(Some(task)) => JsonRpcResponse::success(
                id,
                serde_json::to_value(&task).unwrap_or_else(|_| json!({})),
            ),
            Ok(None) => JsonRpcResponse::error(
                id,
                JsonRpcError {
                    code: JSONRPC_TASK_NOT_FOUND,
                    message: format!("task `{task_id}` not found"),
                    data: None,
                },
            ),
            Err(err) => JsonRpcResponse::error(
                id,
                JsonRpcError {
                    code: -32603,
                    message: err.to_string(),
                    data: None,
                },
            ),
        }
    }

    async fn handle_tasks_cancel(&self, id: JsonRpcId, params: Option<Value>) -> JsonRpcResponse {
        let Some(task_id) = extract_task_id(params.as_ref()) else {
            return invalid_params(id, "missing task id");
        };
        match self.store.0.get_task(&task_id).await {
            Ok(Some(mut task)) => {
                task.status.state = TaskState::Canceled;
                task.status.timestamp = Some(Utc::now());
                match self.store.0.save_task(task, None, None).await {
                    Ok(saved) => JsonRpcResponse::success(
                        id,
                        serde_json::to_value(&saved).unwrap_or_else(|_| json!({})),
                    ),
                    Err(err) => JsonRpcResponse::error(
                        id,
                        JsonRpcError {
                            code: -32603,
                            message: err.to_string(),
                            data: None,
                        },
                    ),
                }
            }
            Ok(None) => JsonRpcResponse::error(
                id,
                JsonRpcError {
                    code: JSONRPC_TASK_NOT_FOUND,
                    message: format!("task `{task_id}` not found"),
                    data: None,
                },
            ),
            Err(err) => JsonRpcResponse::error(
                id,
                JsonRpcError {
                    code: -32603,
                    message: err.to_string(),
                    data: None,
                },
            ),
        }
    }

    /// Sends an outbound A2A JSON-RPC `message/send` call with mandatory installation identity headers.
    pub async fn send_outbound(
        &self,
        dto: &OutboundA2aSendDto,
    ) -> Result<OutboundA2aResponse, ServerError> {
        let parsed = Url::parse(&dto.target_url)
            .map_err(|e| ServerError::Validation(format!("invalid A2A targetUrl: {e}")))?;
        if !matches!(parsed.scheme(), "http" | "https") {
            return Err(ServerError::Validation(
                "A2A targetUrl must use http or https".into(),
            ));
        }

        let mut headers = HeaderMap::new();
        let active_identity = self.identity.inject_outbound_headers(&mut headers).await?;

        let mut msg = Message::new(Role::User, vec![Part::text(dto.text.clone())]);
        msg.context_id = dto.context_id.clone();

        let rpc_req = JsonRpcRequest::new(
            JsonRpcId::String(uuid::Uuid::new_v4().to_string()),
            "message/send",
            Some(json!({ "message": msg })),
        );

        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(15))
            .build()
            .map_err(|e| ServerError::Internal(format!("a2a client build failed: {e}")))?;

        let resp = client
            .post(parsed.as_str())
            .headers(headers)
            .json(&rpc_req)
            .send()
            .await
            .map_err(|e| ServerError::Validation(format!("outbound A2A request failed: {e}")))?;

        let status = resp.status().as_u16();
        let body: Value = resp
            .json()
            .await
            .unwrap_or_else(|_| json!({ "rawStatus": status }));

        let _ = self
            .identity
            .record_outbound(OutboundInteractionRecord {
                protocol: "a2a",
                target_host: parsed.host_str(),
                endpoint_path: parsed.path(),
                method: "POST",
                status_code: Some(i32::from(status)),
                task_id: None,
                identity: Some(&active_identity),
            })
            .await;

        Ok(OutboundA2aResponse {
            target_url: dto.target_url.clone(),
            status_code: status,
            injected_org: active_identity.organization_name,
            injected_installation_id: active_identity.installation_id,
            response: body,
        })
    }
}

fn extract_task_id(params: Option<&Value>) -> Option<String> {
    params?
        .get("id")
        .or_else(|| params?.get("taskId"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
}

fn invalid_params(id: JsonRpcId, msg: &str) -> JsonRpcResponse {
    JsonRpcResponse::error(
        id,
        JsonRpcError {
            code: JSONRPC_INVALID_PARAMS,
            message: msg.to_owned(),
            data: None,
        },
    )
}
