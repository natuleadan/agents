use std::collections::BTreeMap;
use std::sync::Arc;

use agents_protocol::{EventEnvelope, InteractionMode, ThirdPartyProvider, TokenPair};
use serde_json::json;

use crate::core::ClientError;
use crate::core::backend::{
    ActionReceipt, BoxFuture, ControlAction, HttpMethod, HttpRequest, ProviderCapabilities,
    SandboxProvider, SessionSummary, TaskSubmission, Transport,
};

/// Direct Third-Party Mode adapter with honest capabilities and zero silent fallback.
pub struct ThirdPartyAdapter {
    provider: ThirdPartyProvider,
    endpoint: String,
    transport: Arc<dyn Transport>,
}

impl ThirdPartyAdapter {
    #[must_use]
    pub fn new(
        provider: ThirdPartyProvider,
        endpoint: impl Into<String>,
        transport: Arc<dyn Transport>,
    ) -> Self {
        Self {
            provider,
            endpoint: endpoint.into().trim_end_matches('/').to_owned(),
            transport,
        }
    }

    /// Declares exact honest capabilities supported by each external provider.
    #[must_use]
    pub fn capabilities_for(provider: &ThirdPartyProvider) -> ProviderCapabilities {
        match provider {
            ThirdPartyProvider::UpstashBox => ProviderCapabilities {
                supports_fencing: false,
                supports_live_steering: false,
                supports_preview_routes: true,
                supports_swarm: false,
                supports_autocron: false,
                supports_ws_stream: false,
                supports_replay: false,
            },
            ThirdPartyProvider::Daytona => ProviderCapabilities {
                supports_fencing: false,
                supports_live_steering: true,
                supports_preview_routes: true,
                supports_swarm: false,
                supports_autocron: false,
                supports_ws_stream: true,
                supports_replay: false,
            },
            ThirdPartyProvider::DeepInfra => ProviderCapabilities {
                supports_fencing: false,
                supports_live_steering: false,
                supports_preview_routes: false,
                supports_swarm: false,
                supports_autocron: false,
                supports_ws_stream: false,
                supports_replay: false,
            },
            ThirdPartyProvider::Instabox | ThirdPartyProvider::Unknown(_) => ProviderCapabilities {
                supports_fencing: false,
                supports_live_steering: false,
                supports_preview_routes: true,
                supports_swarm: false,
                supports_autocron: false,
                supports_ws_stream: false,
                supports_replay: false,
            },
        }
    }

    fn auth_headers(token: &str, idempotency_key: Option<&str>) -> BTreeMap<String, String> {
        let mut h = BTreeMap::new();
        h.insert("content-type".to_owned(), "application/json".to_owned());
        h.insert("authorization".to_owned(), format!("Bearer {token}"));
        if let Some(key) = idempotency_key {
            h.insert("idempotency-key".to_owned(), key.to_owned());
        }
        h
    }
}

impl SandboxProvider for ThirdPartyAdapter {
    fn name(&self) -> &str {
        self.provider.as_str()
    }

    fn capabilities(&self) -> ProviderCapabilities {
        Self::capabilities_for(&self.provider)
    }

    fn login<'a>(
        &'a self,
        _email: &'a str,
        api_key: &'a str,
    ) -> BoxFuture<'a, Result<TokenPair, ClientError>> {
        Box::pin(async move {
            if api_key.trim().is_empty() {
                return Err(ClientError::Auth(
                    "third-party provider requires non-empty API key".to_owned(),
                ));
            }
            Ok(TokenPair {
                access_token: api_key.to_owned(),
                refresh_token: String::new(),
                token_type: "Bearer".to_owned(),
                expires_in_seconds: 86_400,
            })
        })
    }

    fn refresh<'a>(
        &'a self,
        _refresh_token: &'a str,
    ) -> BoxFuture<'a, Result<TokenPair, ClientError>> {
        let provider = self.provider.as_str().to_owned();
        Box::pin(async move {
            Err(ClientError::UnsupportedCapability {
                provider,
                capability: "token_refresh".to_owned(),
            })
        })
    }

    fn create_session<'a>(
        &'a self,
        token: &'a str,
        title: &'a str,
        mode: InteractionMode,
    ) -> BoxFuture<'a, Result<SessionSummary, ClientError>> {
        Box::pin(async move {
            let req = HttpRequest {
                method: HttpMethod::Post,
                url: format!("{}/boxes", self.endpoint),
                headers: Self::auth_headers(token, None),
                body: Some(json!({ "name": title }).to_string()),
            };
            let resp = self.transport.execute(req).await?;
            if !(200..300).contains(&resp.status) {
                return Err(ClientError::HttpStatus {
                    status: resp.status,
                    body: resp.body,
                });
            }
            let val: serde_json::Value = serde_json::from_str(&resp.body)?;
            let id = val["id"]
                .as_str()
                .ok_or_else(|| ClientError::Transport("missing third-party sandbox id".to_owned()))?
                .to_owned();
            Ok(SessionSummary {
                id,
                title: title.to_owned(),
                mode,
            })
        })
    }

    fn submit_task<'a>(
        &'a self,
        token: &'a str,
        session_id: &'a str,
        prompt: &'a str,
        _mode: InteractionMode,
    ) -> BoxFuture<'a, Result<TaskSubmission, ClientError>> {
        Box::pin(async move {
            let req = HttpRequest {
                method: HttpMethod::Post,
                url: format!("{}/boxes/{session_id}/exec", self.endpoint),
                headers: Self::auth_headers(token, None),
                body: Some(json!({ "prompt": prompt }).to_string()),
            };
            let resp = self.transport.execute(req).await?;
            if !(200..300).contains(&resp.status) {
                return Err(ClientError::HttpStatus {
                    status: resp.status,
                    body: resp.body,
                });
            }
            let val: serde_json::Value = serde_json::from_str(&resp.body)?;
            let task_id = val["executionId"]
                .as_str()
                .or_else(|| val["id"].as_str())
                .ok_or_else(|| ClientError::Transport("missing third-party task id".to_owned()))?
                .to_owned();
            Ok(TaskSubmission {
                task_id,
                session_id: session_id.to_owned(),
                prompt: prompt.to_owned(),
            })
        })
    }

    fn send_control<'a>(
        &'a self,
        token: &'a str,
        task_id: &'a str,
        action: ControlAction,
        idempotency_key: &'a str,
    ) -> BoxFuture<'a, Result<ActionReceipt, ClientError>> {
        Box::pin(async move {
            if action != ControlAction::Stop {
                return Err(ClientError::UnsupportedCapability {
                    provider: self.provider.as_str().to_owned(),
                    capability: action.as_str().to_owned(),
                });
            }
            let req = HttpRequest {
                method: HttpMethod::Post,
                url: format!("{}/executions/{task_id}/stop", self.endpoint),
                headers: Self::auth_headers(token, Some(idempotency_key)),
                body: None,
            };
            let resp = self.transport.execute(req).await?;
            if !(200..300).contains(&resp.status) {
                return Err(ClientError::HttpStatus {
                    status: resp.status,
                    body: resp.body,
                });
            }
            Ok(ActionReceipt {
                idempotency_key: idempotency_key.to_owned(),
                accepted: true,
                status_note: "stopped".to_owned(),
            })
        })
    }

    fn steer_task<'a>(
        &'a self,
        token: &'a str,
        task_id: &'a str,
        content: &'a str,
    ) -> BoxFuture<'a, Result<(), ClientError>> {
        Box::pin(async move {
            if !self.capabilities().supports_live_steering {
                return Err(ClientError::UnsupportedCapability {
                    provider: self.provider.as_str().to_owned(),
                    capability: "live_steering".to_owned(),
                });
            }
            let req = HttpRequest {
                method: HttpMethod::Post,
                url: format!("{}/executions/{task_id}/steer", self.endpoint),
                headers: Self::auth_headers(token, None),
                body: Some(json!({ "content": content }).to_string()),
            };
            let resp = self.transport.execute(req).await?;
            if !(200..300).contains(&resp.status) {
                return Err(ClientError::HttpStatus {
                    status: resp.status,
                    body: resp.body,
                });
            }
            Ok(())
        })
    }

    fn replay_events<'a>(
        &'a self,
        _token: &'a str,
        _session_id: &'a str,
        _after_seq: u64,
    ) -> BoxFuture<'a, Result<Vec<EventEnvelope>, ClientError>> {
        let provider = self.provider.as_str().to_owned();
        Box::pin(async move {
            Err(ClientError::UnsupportedCapability {
                provider,
                capability: "sequence_replay".to_owned(),
            })
        })
    }
}
