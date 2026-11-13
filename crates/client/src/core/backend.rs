use std::collections::BTreeMap;
use std::future::Future;
use std::pin::Pin;

use agents_protocol::{
    EventEnvelope, InteractionMode, IsolationKind, RuntimeKind, SetupMode, ThirdPartyProvider,
    TokenPair,
};
use serde::{Deserialize, Serialize};

use super::ClientError;

/// Heap-allocated future abstraction avoiding proc-macro overhead across all 6 targets.
pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// Operating connection architecture chosen by the user.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "mode", rename_all = "snake_case")]
pub enum BackendMode {
    Server {
        base_url: String,
    },
    ThirdParty {
        provider: ThirdPartyProvider,
        endpoint: String,
    },
}

/// Complete resolved target passed into [`SandboxProvider`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BackendTarget {
    pub setup_mode: SetupMode,
    pub mode: BackendMode,
    pub runtime: RuntimeKind,
    pub isolation: IsolationKind,
}

impl BackendTarget {
    #[must_use]
    pub fn local_server(base_url: impl Into<String>) -> Self {
        Self {
            setup_mode: SetupMode::Local,
            mode: BackendMode::Server {
                base_url: base_url.into(),
            },
            runtime: RuntimeKind::Docker,
            isolation: IsolationKind::Openshell,
        }
    }

    #[must_use]
    pub fn remote_server(base_url: impl Into<String>) -> Self {
        Self {
            setup_mode: SetupMode::Remote,
            mode: BackendMode::Server {
                base_url: base_url.into(),
            },
            runtime: RuntimeKind::RemoteVps,
            isolation: IsolationKind::Openshell,
        }
    }

    #[must_use]
    pub fn third_party(provider: ThirdPartyProvider, endpoint: impl Into<String>) -> Self {
        Self {
            setup_mode: SetupMode::Remote,
            mode: BackendMode::ThirdParty {
                provider: provider.clone(),
                endpoint: endpoint.into(),
            },
            runtime: RuntimeKind::ThirdParty(provider),
            isolation: IsolationKind::Native,
        }
    }

    #[must_use]
    pub fn effective_profile(&self) -> EffectiveProfile {
        let scope = match self.setup_mode {
            SetupMode::Local => "local",
            SetupMode::Remote => "remote",
        };
        let detail = match &self.mode {
            BackendMode::Server { .. } => {
                if self.setup_mode == SetupMode::Local {
                    format!("{} + {}", self.runtime.label(), self.isolation.as_str())
                } else {
                    "server".to_owned()
                }
            }
            BackendMode::ThirdParty { provider, .. } => provider.as_str().to_owned(),
        };
        EffectiveProfile {
            label: format!("{scope} · {detail}"),
            setup_mode: self.setup_mode,
        }
    }
}

/// Visible effective profile pill shown continuously across TUI, desktop, mobile, and web UI.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EffectiveProfile {
    pub label: String,
    pub setup_mode: SetupMode,
}

/// Honest provider capability matrix; unsupported calls fail closed without silent downgrade.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderCapabilities {
    pub supports_fencing: bool,
    pub supports_live_steering: bool,
    pub supports_preview_routes: bool,
    pub supports_swarm: bool,
    pub supports_autocron: bool,
    pub supports_ws_stream: bool,
    pub supports_replay: bool,
}

impl ProviderCapabilities {
    #[must_use]
    pub const fn full_server() -> Self {
        Self {
            supports_fencing: true,
            supports_live_steering: true,
            supports_preview_routes: true,
            supports_swarm: true,
            supports_autocron: true,
            supports_ws_stream: true,
            supports_replay: true,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ControlAction {
    Pause,
    Resume,
    Stop,
    Retry,
}

impl ControlAction {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Pause => "pause",
            Self::Resume => "resume",
            Self::Stop => "stop",
            Self::Retry => "retry",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionSummary {
    pub id: String,
    pub title: String,
    pub mode: InteractionMode,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TaskSubmission {
    pub task_id: String,
    pub session_id: String,
    pub prompt: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActionReceipt {
    pub idempotency_key: String,
    pub accepted: bool,
    pub status_note: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HttpMethod {
    Get,
    Post,
    Put,
    Patch,
    Delete,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HttpRequest {
    pub method: HttpMethod,
    pub url: String,
    pub headers: BTreeMap<String, String>,
    pub body: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HttpResponse {
    pub status: u16,
    pub body: String,
}

/// Pluggable HTTP transport supporting reqwest on desktop targets and deterministic mocks/WASM.
pub trait Transport: Send + Sync {
    fn execute<'a>(&'a self, req: HttpRequest) -> BoxFuture<'a, Result<HttpResponse, ClientError>>;
}

/// Unified async provider contract implemented by Server Mode and Third-Party Mode adapters.
pub trait SandboxProvider: Send + Sync {
    fn name(&self) -> &str;
    fn capabilities(&self) -> ProviderCapabilities;

    fn login<'a>(
        &'a self,
        email: &'a str,
        password: &'a str,
    ) -> BoxFuture<'a, Result<TokenPair, ClientError>>;

    fn refresh<'a>(
        &'a self,
        refresh_token: &'a str,
    ) -> BoxFuture<'a, Result<TokenPair, ClientError>>;

    fn create_session<'a>(
        &'a self,
        token: &'a str,
        title: &'a str,
        mode: InteractionMode,
    ) -> BoxFuture<'a, Result<SessionSummary, ClientError>>;

    fn submit_task<'a>(
        &'a self,
        token: &'a str,
        session_id: &'a str,
        prompt: &'a str,
        mode: InteractionMode,
    ) -> BoxFuture<'a, Result<TaskSubmission, ClientError>>;

    fn send_control<'a>(
        &'a self,
        token: &'a str,
        task_id: &'a str,
        action: ControlAction,
        idempotency_key: &'a str,
    ) -> BoxFuture<'a, Result<ActionReceipt, ClientError>>;

    fn steer_task<'a>(
        &'a self,
        token: &'a str,
        task_id: &'a str,
        content: &'a str,
    ) -> BoxFuture<'a, Result<(), ClientError>>;

    fn replay_events<'a>(
        &'a self,
        token: &'a str,
        session_id: &'a str,
        after_seq: u64,
    ) -> BoxFuture<'a, Result<Vec<EventEnvelope>, ClientError>>;
}
