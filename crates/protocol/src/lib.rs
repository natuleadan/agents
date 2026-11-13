//! Dependency-free shared communication protocol, named codecs, and domain DTOs.

#![forbid(unsafe_code)]

pub mod artifact;
pub mod auth;
pub mod node;
pub mod preview;
pub mod rpc;
pub mod sandbox;
pub mod schema;
pub mod stream;
pub mod swarm;
pub mod task;
pub mod version;
pub mod workflow;

pub use artifact::{ArtifactEntry, ArtifactManifest, ArtifactValidationError, is_valid_sha256_hex};
pub use auth::{
    ApiKeyMetadata, ApiKeyScope, CreateApiKeyRequest, LoginRequest, RefreshRequest, TokenPair,
    UserClaims,
};
pub use node::{BoxerNodeAnnouncement, NodeStatus, PreviewRouteSync};
pub use preview::TraefikPreviewRoute;
pub use rpc::{
    CursorPage, ExecutionScope, METHOD_CATALOG, PageRequest, RequestId, RpcError, RpcErrorCode,
    RpcMethodSpec, RpcNotification, RpcRequest, RpcResponse, TraceContext, TriStateFilter,
};
pub use sandbox::{
    ExecOptions, IsolationKind, RuntimeKind, SandboxProfile, SandboxProfileError, SetupMode,
    ThirdPartyProvider,
};
pub use schema::{ProtocolSchemaBundle, SandboxMatrixEntry, export_protocol_schema};
pub use stream::{EventEnvelope, StreamPayload, TaskStreamEvent, ToolPhase};
pub use swarm::{AgentHandoff, PermissionError, PermissionMask, SwarmMessage, SwarmTopic};
pub use task::{
    FencingToken, FollowUpMessage, InteractionMode, SteeringMessage, StopReason, TaskLease,
    TaskRequest, TaskResult, TaskState, TurnDecision, WorkerStatus,
};
pub use version::{API_GENERATION, Capability, CodecId, HandshakeRequest, HandshakeResponse};
pub use workflow::{AutocronSchedule, TriggerKind, WorkflowDag, WorkflowEdge, WorkflowNode};
