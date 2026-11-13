//! Multi-provider model endpoint registry supporting Ollama, LMStudio, OpenAI,
//! Anthropic, DeepInfra, and infinite custom `provider_endpoint:<id>` targets.

use std::{collections::HashMap, sync::Arc};

use agents_protocol::StopReason;
use bytes::Bytes;
use http::{Request, header};
use http_body_util::{BodyExt, Full};
use hyper_util::{
    client::legacy::{Client, connect::HttpConnector},
    rt::TokioExecutor,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::sync::Mutex;

use crate::{
    error::BoxerError,
    leases::BoxFuture,
    runtime::{engine::ToolCallRequest, session_tree::SessionEntry},
};

/// Supported LLM inference backend adapters.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ProviderKind {
    Ollama,
    LmStudio,
    OpenAi,
    Anthropic,
    DeepInfra,
    Custom(String),
}

impl ProviderKind {
    #[must_use]
    pub fn as_str(&self) -> &str {
        match self {
            Self::Ollama => "ollama",
            Self::LmStudio => "lmstudio",
            Self::OpenAi => "openai",
            Self::Anthropic => "anthropic",
            Self::DeepInfra => "deepinfra",
            Self::Custom(id) => id.as_str(),
        }
    }
}

/// Configuration for a named provider endpoint (`provider_endpoint:<id>`).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderEndpointConfig {
    pub id: String,
    pub kind: ProviderKind,
    pub base_url: String,
    pub model: String,
    #[serde(default)]
    pub api_key_env: Option<String>,
    #[serde(default)]
    pub default_headers: HashMap<String, String>,
}

impl ProviderEndpointConfig {
    /// Canonical config key used when referencing dynamic custom endpoints.
    #[must_use]
    pub fn registry_key(&self) -> String {
        if self.id.starts_with("provider_endpoint:") {
            self.id.clone()
        } else {
            format!("provider_endpoint:{}", self.id)
        }
    }
}

/// Registry holding infinite custom `provider_endpoint:<id>` and built-in endpoints.
#[derive(Debug, Clone, Default)]
pub struct ProviderRegistry {
    endpoints: HashMap<String, ProviderEndpointConfig>,
}

impl ProviderRegistry {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register(&mut self, cfg: ProviderEndpointConfig) {
        let key = cfg.registry_key();
        self.endpoints.insert(cfg.id.clone(), cfg.clone());
        self.endpoints.insert(key, cfg);
    }

    #[must_use]
    pub fn resolve(&self, id: &str) -> Option<&ProviderEndpointConfig> {
        self.endpoints
            .get(id)
            .or_else(|| self.endpoints.get(&format!("provider_endpoint:{id}")))
    }

    #[must_use]
    pub fn list(&self) -> Vec<&ProviderEndpointConfig> {
        self.endpoints
            .iter()
            .filter(|(k, _)| k.starts_with("provider_endpoint:"))
            .map(|(_, v)| v)
            .collect()
    }
}

/// Normalized request forwarded to a model provider for one inner-loop inference step.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelTurnRequest {
    pub session_id: String,
    pub turn_index: u32,
    pub endpoint_id: String,
    pub entries: Vec<SessionEntry>,
    pub available_tools: Vec<String>,
    pub json_schema_mode: bool,
}

/// Normalized model inference output driving tool batches or loop termination.
#[derive(Debug, Clone)]
pub struct ModelTurnResponse {
    pub assistant_text: String,
    pub tool_calls: Vec<ToolCallRequest>,
    pub stop_reason: StopReason,
    pub usage_tokens: u64,
}

/// Model client abstraction decouples inference transport from loop logic.
pub trait ModelClient: Send + Sync + 'static {
    fn complete_turn<'a>(
        &'a self,
        req: ModelTurnRequest,
    ) -> BoxFuture<'a, Result<ModelTurnResponse, BoxerError>>;
}

/// HTTP model adapter dispatching normalized requests to configured provider endpoints.
#[derive(Clone)]
pub struct HttpProviderClient {
    registry: Arc<ProviderRegistry>,
    http: Client<HttpConnector, Full<Bytes>>,
}

impl HttpProviderClient {
    #[must_use]
    pub fn new(registry: Arc<ProviderRegistry>) -> Self {
        let mut connector = HttpConnector::new();
        connector.enforce_http(false);
        let http = Client::builder(TokioExecutor::new()).build(connector);
        Self { registry, http }
    }

    fn build_provider_payload(cfg: &ProviderEndpointConfig, req: &ModelTurnRequest) -> Value {
        let messages: Vec<Value> = req
            .entries
            .iter()
            .map(|entry| json!({ "role": entry.role_label(), "content": entry.summary_text() }))
            .collect();
        match cfg.kind {
            ProviderKind::Anthropic => json!({
                "model": cfg.model,
                "max_tokens": 2048,
                "messages": messages,
            }),
            ProviderKind::Ollama => json!({
                "model": cfg.model,
                "stream": false,
                "messages": messages,
            }),
            ProviderKind::LmStudio
            | ProviderKind::OpenAi
            | ProviderKind::DeepInfra
            | ProviderKind::Custom(_) => json!({
                "model": cfg.model,
                "messages": messages,
            }),
        }
    }

    fn target_url(cfg: &ProviderEndpointConfig) -> String {
        let base = cfg.base_url.trim_end_matches('/');
        match cfg.kind {
            ProviderKind::Ollama => format!("{base}/api/chat"),
            ProviderKind::Anthropic => format!("{base}/v1/messages"),
            ProviderKind::LmStudio
            | ProviderKind::OpenAi
            | ProviderKind::DeepInfra
            | ProviderKind::Custom(_) => format!("{base}/v1/chat/completions"),
        }
    }
}

impl ModelClient for HttpProviderClient {
    fn complete_turn<'a>(
        &'a self,
        req: ModelTurnRequest,
    ) -> BoxFuture<'a, Result<ModelTurnResponse, BoxerError>> {
        Box::pin(async move {
            let cfg = self
                .registry
                .resolve(&req.endpoint_id)
                .ok_or_else(|| {
                    BoxerError::Config(format!(
                        "provider endpoint `{}` is not registered",
                        req.endpoint_id
                    ))
                })?
                .clone();

            let url = Self::target_url(&cfg);
            let payload = Self::build_provider_payload(&cfg, &req);
            let body_bytes = serde_json::to_vec(&payload)
                .map_err(|e| BoxerError::ExecutionFailed(format!("provider json error: {e}")))?;

            let mut builder = Request::post(&url).header(header::CONTENT_TYPE, "application/json");
            for (k, v) in &cfg.default_headers {
                builder = builder.header(k.as_str(), v.as_str());
            }
            if let Some(env_name) = &cfg.api_key_env
                && let Ok(secret) = std::env::var(env_name)
                && !secret.trim().is_empty()
            {
                match cfg.kind {
                    ProviderKind::Anthropic => {
                        builder = builder.header("x-api-key", secret);
                    }
                    _ => {
                        builder = builder.header(header::AUTHORIZATION, format!("Bearer {secret}"));
                    }
                }
            }

            let http_req = builder
                .body(Full::new(Bytes::from(body_bytes)))
                .map_err(|e| BoxerError::ExecutionFailed(format!("invalid provider uri: {e}")))?;

            let resp = self.http.request(http_req).await.map_err(|e| {
                BoxerError::ExecutionFailed(format!("provider request failed: {e}"))
            })?;
            let status = resp.status();
            let raw = resp
                .into_body()
                .collect()
                .await
                .map_err(|e| BoxerError::ExecutionFailed(format!("provider read error: {e}")))?
                .to_bytes();

            if !status.is_success() {
                return Err(BoxerError::ExecutionFailed(format!(
                    "provider HTTP {}: {}",
                    status.as_u16(),
                    String::from_utf8_lossy(&raw)
                )));
            }

            let parsed: Value = serde_json::from_slice(&raw)
                .map_err(|e| BoxerError::ExecutionFailed(format!("invalid provider json: {e}")))?;

            let assistant_text = parsed
                .pointer("/choices/0/message/content")
                .or_else(|| parsed.pointer("/message/content"))
                .or_else(|| parsed.pointer("/content/0/text"))
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned();

            Ok(ModelTurnResponse {
                assistant_text,
                tool_calls: Vec::new(),
                stop_reason: StopReason::EndTurn,
                usage_tokens: parsed
                    .pointer("/usage/total_tokens")
                    .and_then(Value::as_u64)
                    .unwrap_or(0),
            })
        })
    }
}

/// Deterministic scripted model client recording exact prompts received in tests.
#[derive(Default)]
pub struct ScriptedModelClient {
    responses: Mutex<Vec<Result<ModelTurnResponse, BoxerError>>>,
    seen_requests: Mutex<Vec<ModelTurnRequest>>,
}

impl ScriptedModelClient {
    #[must_use]
    pub fn new(mut responses: Vec<Result<ModelTurnResponse, BoxerError>>) -> Arc<Self> {
        responses.reverse();
        Arc::new(Self {
            responses: Mutex::new(responses),
            seen_requests: Mutex::new(Vec::new()),
        })
    }

    pub async fn seen_requests(&self) -> Vec<ModelTurnRequest> {
        self.seen_requests.lock().await.clone()
    }
}

impl ModelClient for ScriptedModelClient {
    fn complete_turn<'a>(
        &'a self,
        req: ModelTurnRequest,
    ) -> BoxFuture<'a, Result<ModelTurnResponse, BoxerError>> {
        Box::pin(async move {
            self.seen_requests.lock().await.push(req);
            self.responses.lock().await.pop().unwrap_or_else(|| {
                Ok(ModelTurnResponse {
                    assistant_text: "done".to_owned(),
                    tool_calls: Vec::new(),
                    stop_reason: StopReason::EndTurn,
                    usage_tokens: 1,
                })
            })
        })
    }
}
