pub mod third_party;

use std::collections::{BTreeMap, VecDeque};
use std::sync::{Arc, Mutex};

use agents_protocol::{EventEnvelope, InteractionMode, TokenPair};
use serde_json::json;

use crate::core::ClientError;
use crate::core::backend::{
    ActionReceipt, BoxFuture, ControlAction, HttpMethod, HttpRequest, HttpResponse,
    ProviderCapabilities, SandboxProvider, SessionSummary, TaskSubmission, Transport,
};

pub use third_party::ThirdPartyAdapter;

/// Deterministic queued HTTP responder for hermetic unit/integration tests and offline sandboxes.
#[derive(Clone, Default)]
pub struct MockTransport {
    recorded: Arc<Mutex<Vec<HttpRequest>>>,
    responses: Arc<Mutex<VecDeque<Result<HttpResponse, ClientError>>>>,
}

impl MockTransport {
    #[must_use]
    pub fn with_responses(responses: Vec<HttpResponse>) -> Self {
        Self {
            recorded: Arc::new(Mutex::new(Vec::new())),
            responses: Arc::new(Mutex::new(responses.into_iter().map(Ok).collect())),
        }
    }

    pub fn push_response(&self, resp: HttpResponse) {
        if let Ok(mut guard) = self.responses.lock() {
            guard.push_back(Ok(resp));
        }
    }

    #[must_use]
    pub fn recorded_requests(&self) -> Vec<HttpRequest> {
        self.recorded.lock().map(|g| g.clone()).unwrap_or_default()
    }
}

impl Transport for MockTransport {
    fn execute<'a>(&'a self, req: HttpRequest) -> BoxFuture<'a, Result<HttpResponse, ClientError>> {
        Box::pin(async move {
            if let Ok(mut rec) = self.recorded.lock() {
                rec.push(req);
            }
            let next = self
                .responses
                .lock()
                .map_err(|_| ClientError::Transport("mock lock poisoned".to_owned()))?
                .pop_front();
            match next {
                Some(res) => res,
                None => Err(ClientError::Transport("no queued mock response".to_owned())),
            }
        })
    }
}

/// Native HTTPS transport using `reqwest` on desktop targets.
#[cfg(any(target_os = "linux", target_os = "macos", target_os = "windows"))]
#[derive(Clone)]
pub struct NativeReqwestTransport {
    client: reqwest::Client,
}

#[cfg(any(target_os = "linux", target_os = "macos", target_os = "windows"))]
impl Default for NativeReqwestTransport {
    fn default() -> Self {
        Self {
            client: reqwest::Client::new(),
        }
    }
}

#[cfg(any(target_os = "linux", target_os = "macos", target_os = "windows"))]
impl Transport for NativeReqwestTransport {
    fn execute<'a>(&'a self, req: HttpRequest) -> BoxFuture<'a, Result<HttpResponse, ClientError>> {
        Box::pin(async move {
            let method = match req.method {
                HttpMethod::Get => reqwest::Method::GET,
                HttpMethod::Post => reqwest::Method::POST,
                HttpMethod::Put => reqwest::Method::PUT,
                HttpMethod::Patch => reqwest::Method::PATCH,
                HttpMethod::Delete => reqwest::Method::DELETE,
            };
            let mut builder = self.client.request(method, &req.url);
            for (k, v) in &req.headers {
                builder = builder.header(k, v);
            }
            if let Some(body) = req.body {
                builder = builder
                    .header("content-type", "application/json")
                    .body(body);
            }
            let response = builder
                .send()
                .await
                .map_err(|e| ClientError::Transport(e.to_string()))?;
            let status = response.status().as_u16();
            let body = response
                .text()
                .await
                .map_err(|e| ClientError::Transport(e.to_string()))?;
            Ok(HttpResponse { status, body })
        })
    }
}

/// Self-hosted `agents-server` provider implementing full Server Mode capabilities.
pub struct ServerModeProvider {
    base_url: String,
    transport: Arc<dyn Transport>,
}

impl ServerModeProvider {
    #[must_use]
    pub fn new(base_url: impl Into<String>, transport: Arc<dyn Transport>) -> Self {
        let base = base_url.into().trim_end_matches('/').to_owned();
        Self {
            base_url: base,
            transport,
        }
    }

    fn json_headers(
        token: Option<&str>,
        idempotency_key: Option<&str>,
    ) -> BTreeMap<String, String> {
        let mut headers = BTreeMap::new();
        headers.insert("content-type".to_owned(), "application/json".to_owned());
        if let Some(t) = token {
            headers.insert("authorization".to_owned(), format!("Bearer {t}"));
        }
        if let Some(key) = idempotency_key {
            headers.insert("idempotency-key".to_owned(), key.to_owned());
        }
        headers
    }

    fn ensure_ok(resp: HttpResponse) -> Result<String, ClientError> {
        if (200..300).contains(&resp.status) {
            Ok(resp.body)
        } else if resp.status == 401 || resp.status == 403 {
            Err(ClientError::Auth(resp.body))
        } else {
            Err(ClientError::HttpStatus {
                status: resp.status,
                body: resp.body,
            })
        }
    }
}

impl SandboxProvider for ServerModeProvider {
    fn name(&self) -> &str {
        "agents-server"
    }

    fn capabilities(&self) -> ProviderCapabilities {
        ProviderCapabilities::full_server()
    }

    fn login<'a>(
        &'a self,
        email: &'a str,
        password: &'a str,
    ) -> BoxFuture<'a, Result<TokenPair, ClientError>> {
        Box::pin(async move {
            let payload = json!({
                "email": email,
                "password": password,
            });
            let req = HttpRequest {
                method: HttpMethod::Post,
                url: format!("{}/api/v1/auth/login", self.base_url),
                headers: Self::json_headers(None, None),
                body: Some(payload.to_string()),
            };
            let body = Self::ensure_ok(self.transport.execute(req).await?)?;
            Ok(serde_json::from_str(&body)?)
        })
    }

    fn refresh<'a>(
        &'a self,
        refresh_token: &'a str,
    ) -> BoxFuture<'a, Result<TokenPair, ClientError>> {
        Box::pin(async move {
            let payload = json!({
                "refreshToken": refresh_token,
            });
            let req = HttpRequest {
                method: HttpMethod::Post,
                url: format!("{}/api/v1/auth/refresh", self.base_url),
                headers: Self::json_headers(None, None),
                body: Some(payload.to_string()),
            };
            let body = Self::ensure_ok(self.transport.execute(req).await?)?;
            Ok(serde_json::from_str(&body)?)
        })
    }

    fn create_session<'a>(
        &'a self,
        token: &'a str,
        title: &'a str,
        mode: InteractionMode,
    ) -> BoxFuture<'a, Result<SessionSummary, ClientError>> {
        Box::pin(async move {
            let payload = json!({
                "title": title,
                "mode": mode,
            });
            let req = HttpRequest {
                method: HttpMethod::Post,
                url: format!("{}/api/v1/sessions", self.base_url),
                headers: Self::json_headers(Some(token), None),
                body: Some(payload.to_string()),
            };
            let body = Self::ensure_ok(self.transport.execute(req).await?)?;
            let val: serde_json::Value = serde_json::from_str(&body)?;
            let id = val["id"]
                .as_str()
                .ok_or_else(|| ClientError::Transport("missing session id".to_owned()))?
                .to_owned();
            let title_str = val["title"].as_str().unwrap_or(title).to_owned();
            Ok(SessionSummary {
                id,
                title: title_str,
                mode,
            })
        })
    }

    fn submit_task<'a>(
        &'a self,
        token: &'a str,
        session_id: &'a str,
        prompt: &'a str,
        mode: InteractionMode,
    ) -> BoxFuture<'a, Result<TaskSubmission, ClientError>> {
        Box::pin(async move {
            let payload = json!({
                "prompt": prompt,
                "mode": mode,
            });
            let req = HttpRequest {
                method: HttpMethod::Post,
                url: format!("{}/api/v1/sessions/{session_id}/tasks", self.base_url),
                headers: Self::json_headers(Some(token), None),
                body: Some(payload.to_string()),
            };
            let body = Self::ensure_ok(self.transport.execute(req).await?)?;
            let val: serde_json::Value = serde_json::from_str(&body)?;
            let task_id = val["id"]
                .as_str()
                .or_else(|| val["taskId"].as_str())
                .ok_or_else(|| ClientError::Transport("missing task id".to_owned()))?
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
            let path = match action {
                ControlAction::Stop => format!("{}/api/v1/tasks/{task_id}/cancel", self.base_url),
                ControlAction::Pause | ControlAction::Resume | ControlAction::Retry => {
                    format!("{}/api/v1/tasks/{task_id}/steer", self.base_url)
                }
            };
            let body_json = match action {
                ControlAction::Stop => json!({ "reason": "client_requested_stop" }),
                _ => json!({ "content": format!("/{}", action.as_str()) }),
            };
            let req = HttpRequest {
                method: HttpMethod::Post,
                url: path,
                headers: Self::json_headers(Some(token), Some(idempotency_key)),
                body: Some(body_json.to_string()),
            };
            let _ = Self::ensure_ok(self.transport.execute(req).await?)?;
            Ok(ActionReceipt {
                idempotency_key: idempotency_key.to_owned(),
                accepted: true,
                status_note: action.as_str().to_owned(),
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
            let payload = json!({ "content": content });
            let req = HttpRequest {
                method: HttpMethod::Post,
                url: format!("{}/api/v1/tasks/{task_id}/steer", self.base_url),
                headers: Self::json_headers(Some(token), None),
                body: Some(payload.to_string()),
            };
            let _ = Self::ensure_ok(self.transport.execute(req).await?)?;
            Ok(())
        })
    }

    fn replay_events<'a>(
        &'a self,
        token: &'a str,
        session_id: &'a str,
        after_seq: u64,
    ) -> BoxFuture<'a, Result<Vec<EventEnvelope>, ClientError>> {
        Box::pin(async move {
            let req = HttpRequest {
                method: HttpMethod::Get,
                url: format!(
                    "{}/api/v1/sessions/{session_id}/events?after_seq={after_seq}",
                    self.base_url
                ),
                headers: Self::json_headers(Some(token), None),
                body: None,
            };
            let body = Self::ensure_ok(self.transport.execute(req).await?)?;
            Ok(serde_json::from_str(&body)?)
        })
    }
}
