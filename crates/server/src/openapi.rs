//! Centralized OpenAPI 3.1 configuration and component schema registry.

use agents_protocol::{
    ApiKeyMetadata, ApiKeyScope, BoxerNodeAnnouncement, EventEnvelope, ExecOptions, FencingToken,
    FollowUpMessage, InteractionMode, IsolationKind, NodeStatus, PermissionMask, RuntimeKind,
    SandboxProfile, SetupMode, SteeringMessage, StopReason, TaskLease, TaskResult, TaskState,
    TokenPair, TraefikPreviewRoute, TriggerKind, WorkflowDag, WorkflowEdge, WorkflowNode,
};
use ironic::{OpenApiConfig, SecurityScheme};

use crate::modules::{
    audit::{
        dto::{AuditQueryDto, AuditVerificationReport},
        entities::AuditLogRecord,
    },
    auth::{
        dto::{
            CreateApiKeyDto, CreatedApiKeyResponse, LoginDto, RefreshDto, RegisterDto,
            RevokeApiKeyResponse,
        },
        entities::AuthenticatedActor,
    },
    autocron::{
        dto::{CreateScheduleDto, ExecuteScheduleDto, ToggleScheduleDto, TriggerWebhookDto},
        entities::{AutocronRecoveryReport, AutocronScheduleEntity, AutocronTickOutcome},
    },
    gateway::dto::StreamReplayQuery,
    mcp::dto::McpPolicyInfo,
    nodes::{
        dto::{NodeHeartbeatDto, PreviewSyncDto},
        entities::{NodeListResponse, PreviewSyncAck},
    },
    sessions::{
        dto::{CreateSessionDto, ListSessionsQuery, SaveDraftDto, UpdateSessionDto},
        entities::{DraftSnapshot, SessionEntity},
    },
    swarm::{
        dto::{CreateHandoffDto, PutBlackboardDto, SendSwarmMessageDto},
        entities::{MediatedSwarmMessage, SwarmHandoffEntity},
    },
    tasks::{
        dto::{
            CompleteTaskDto, FollowUpDto, LeaseTaskDto, ListTasksQuery, SteerTaskDto,
            SubmitTaskDto, UpdateTaskStateDto,
        },
        entities::TaskEntity,
    },
    workflows::{
        dto::{
            ApproveWorkflowDto, CreateWorkflowDto, PublishRenderSlugDto, TriggerWorkflowDto,
            WebhookPayloadDto,
        },
        entities::{RenderedSlugEntity, WorkflowEntity, WorkflowRunEntity, WorkflowRunStatus},
    },
};

/// Builds and registers all domain component schemas into the OpenAPI 3.1 specification.
#[must_use]
pub fn configure_openapi() -> OpenApiConfig {
    OpenApiConfig::new("agents-server API", env!("CARGO_PKG_VERSION"))
        .description("Distributed agent harness control plane built on Ironic")
        .security_scheme(
            "bearerAuth",
            SecurityScheme::HttpBearer {
                bearer_format: Some("JWT".to_owned()),
            },
        )
        // Protocol schemas
        .schema::<ApiKeyScope>("ApiKeyScope")
        .schema::<TokenPair>("TokenPair")
        .schema::<ApiKeyMetadata>("ApiKeyMetadata")
        .schema::<FencingToken>("FencingToken")
        .schema::<TaskState>("TaskState")
        .schema::<InteractionMode>("InteractionMode")
        .schema::<StopReason>("StopReason")
        .schema::<SetupMode>("SetupMode")
        .schema::<RuntimeKind>("RuntimeKind")
        .schema::<IsolationKind>("IsolationKind")
        .schema::<SandboxProfile>("SandboxProfile")
        .schema::<ExecOptions>("ExecOptions")
        .schema::<TaskLease>("TaskLease")
        .schema::<TaskResult>("TaskResult")
        .schema::<SteeringMessage>("SteeringMessage")
        .schema::<FollowUpMessage>("FollowUpMessage")
        .schema::<BoxerNodeAnnouncement>("BoxerNodeAnnouncement")
        .schema::<EventEnvelope>("EventEnvelope")
        .schema::<TriggerKind>("TriggerKind")
        .schema::<WorkflowNode>("WorkflowNode")
        .schema::<WorkflowEdge>("WorkflowEdge")
        .schema::<WorkflowDag>("WorkflowDag")
        .schema::<PermissionMask>("PermissionMask")
        .schema::<NodeStatus>("NodeStatus")
        .schema::<TraefikPreviewRoute>("TraefikPreviewRoute")
        // Auth schemas
        .schema::<RegisterDto>("RegisterDto")
        .schema::<LoginDto>("LoginDto")
        .schema::<RefreshDto>("RefreshDto")
        .schema::<CreateApiKeyDto>("CreateApiKeyDto")
        .schema::<CreatedApiKeyResponse>("CreatedApiKeyResponse")
        .schema::<RevokeApiKeyResponse>("RevokeApiKeyResponse")
        .schema::<AuthenticatedActor>("AuthenticatedActor")
        // Task schemas
        .schema::<SubmitTaskDto>("SubmitTaskDto")
        .schema::<LeaseTaskDto>("LeaseTaskDto")
        .schema::<UpdateTaskStateDto>("UpdateTaskStateDto")
        .schema::<CompleteTaskDto>("CompleteTaskDto")
        .schema::<SteerTaskDto>("SteerTaskDto")
        .schema::<FollowUpDto>("FollowUpDto")
        .schema::<ListTasksQuery>("ListTasksQuery")
        .schema::<TaskEntity>("TaskEntity")
        // Session schemas
        .schema::<CreateSessionDto>("CreateSessionDto")
        .schema::<UpdateSessionDto>("UpdateSessionDto")
        .schema::<ListSessionsQuery>("ListSessionsQuery")
        .schema::<SaveDraftDto>("SaveDraftDto")
        .schema::<DraftSnapshot>("DraftSnapshot")
        .schema::<SessionEntity>("SessionEntity")
        // Autocron schemas
        .schema::<CreateScheduleDto>("CreateScheduleDto")
        .schema::<ExecuteScheduleDto>("ExecuteScheduleDto")
        .schema::<ToggleScheduleDto>("ToggleScheduleDto")
        .schema::<TriggerWebhookDto>("TriggerWebhookDto")
        .schema::<AutocronScheduleEntity>("AutocronScheduleEntity")
        .schema::<AutocronTickOutcome>("AutocronTickOutcome")
        .schema::<AutocronRecoveryReport>("AutocronRecoveryReport")
        // Workflow schemas
        .schema::<CreateWorkflowDto>("CreateWorkflowDto")
        .schema::<ApproveWorkflowDto>("ApproveWorkflowDto")
        .schema::<PublishRenderSlugDto>("PublishRenderSlugDto")
        .schema::<TriggerWorkflowDto>("TriggerWorkflowDto")
        .schema::<WebhookPayloadDto>("WebhookPayloadDto")
        .schema::<WorkflowEntity>("WorkflowEntity")
        .schema::<WorkflowRunStatus>("WorkflowRunStatus")
        .schema::<WorkflowRunEntity>("WorkflowRunEntity")
        .schema::<RenderedSlugEntity>("RenderedSlugEntity")
        // Swarm schemas
        .schema::<CreateHandoffDto>("CreateHandoffDto")
        .schema::<SendSwarmMessageDto>("SendSwarmMessageDto")
        .schema::<PutBlackboardDto>("PutBlackboardDto")
        .schema::<SwarmHandoffEntity>("SwarmHandoffEntity")
        .schema::<MediatedSwarmMessage>("MediatedSwarmMessage")
        // Node schemas
        .schema::<NodeHeartbeatDto>("NodeHeartbeatDto")
        .schema::<PreviewSyncDto>("PreviewSyncDto")
        .schema::<NodeListResponse>("NodeListResponse")
        .schema::<PreviewSyncAck>("PreviewSyncAck")
        // Audit schemas
        .schema::<AuditQueryDto>("AuditQueryDto")
        .schema::<AuditVerificationReport>("AuditVerificationReport")
        .schema::<AuditLogRecord>("AuditLogRecord")
        // Gateway & MCP schemas
        .schema::<McpPolicyInfo>("McpPolicyInfo")
        .schema::<StreamReplayQuery>("StreamReplayQuery")
}
