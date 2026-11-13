pub mod adapters;
pub mod backend;
pub mod state;
pub mod store;

use thiserror::Error;

/// Unified error type returned by client storage, network adapters, and UI reducers.
#[derive(Debug, Error)]
pub enum ClientError {
    #[error("authentication failed: {0}")]
    Auth(String),
    #[error("transport error: {0}")]
    Transport(String),
    #[error("remote status {status}: {body}")]
    HttpStatus { status: u16, body: String },
    #[error("serialization error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("capability `{capability}` is unsupported by provider `{provider}`")]
    UnsupportedCapability {
        provider: String,
        capability: String,
    },
    #[error("offline mode active: {0}")]
    Offline(String),
    #[error("storage error: {0}")]
    Storage(String),
}

pub use adapters::{MockTransport, ServerModeProvider, ThirdPartyAdapter};
pub use backend::{
    ActionReceipt, BackendMode, BackendTarget, BoxFuture, ControlAction, EffectiveProfile,
    HttpMethod, HttpRequest, HttpResponse, ProviderCapabilities, SandboxProvider, SessionSummary,
    TaskSubmission, Transport,
};
pub use state::{
    ActiveView, AppAction, AppState, AttachmentEntry, AuthState, ClusterNodeEntry, ContextMeter,
    DiffHunk, DiffLine, DiffLineKind, LoginFormState, NotificationCue, PreviewEntry,
    SubagentDrawerMessage, SwarmForkEntry, ThinkingEntry, UpdateInfo, WorkerEntry, dispatch,
};
pub use store::{
    ChunkCoalescer, CoalesceOutcome, DraftRecord, DraftStore, IdempotentActionQueue,
    InMemorySecretStore, OfflineStartupState, QueuedAction, ScopedKeychainStore, SecretStore,
};
