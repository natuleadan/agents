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
        dto::{NodeHeartbeatDto, NodeRegistrationDto, NodeRegistrationResponse, PreviewSyncDto},
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
        .json_path("/openapi.raw.json")
        .description("Distributed agent harness control plane built on Ironic")
        .security_scheme(
            "bearerAuth",
            SecurityScheme::HttpBearer {
                bearer_format: Some("JWT".to_owned()),
            },
        )
        .security_scheme(
            "apiKeyAuth",
            SecurityScheme::ApiKey {
                name: "X-API-Key".to_owned(),
                location: "header".to_owned(),
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
        .schema::<crate::modules::swarm::dto::BlackboardEntryResponse>("BlackboardEntryResponse")
        // Node schemas
        .schema::<NodeHeartbeatDto>("NodeHeartbeatDto")
        .schema::<NodeRegistrationDto>("NodeRegistrationDto")
        .schema::<NodeRegistrationResponse>("NodeRegistrationResponse")
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
        // Standard error envelope
        .schema::<crate::infra::error::ErrorResponse>("ErrorResponse")
        // System and health schemas
        .schema::<CompositeHealthResponse>("CompositeHealthResponse")
        .schema::<LivenessResponse>("LivenessResponse")
        .schema::<SystemVersionResponse>("SystemVersionResponse")
        .schema::<MetricsScrapeResponse>("MetricsScrapeResponse")
        .schema::<PaginatedPageResponse>("PaginatedPageResponse")
        .schema::<crate::infra::observability::SystemLogsResponse>("SystemLogsResponse")
}

/// Health check detail for a single system dependency.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, ironic::OpenApiSchema)]
pub struct DependencyHealthCheck {
    /// Component status ('ok', 'degraded', 'unhealthy').
    pub status: String,
    /// Optional human-readable diagnostic message.
    pub message: Option<String>,
}

/// Composite health response aggregating database, cache, and message bus indicators.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, ironic::OpenApiSchema)]
pub struct CompositeHealthResponse {
    /// Overall system status ('ok', 'degraded', 'unhealthy').
    pub status: String,
    /// Detailed status per registered health indicator.
    pub checks: std::collections::HashMap<String, DependencyHealthCheck>,
}

/// Daemon process liveness probe response.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, ironic::OpenApiSchema)]
pub struct LivenessResponse {
    /// Process status (always 'alive').
    pub status: String,
}

/// System build and runtime version metadata.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, ironic::OpenApiSchema)]
pub struct SystemVersionResponse {
    /// Semantic release version.
    pub version: String,
    /// Git commit revision hash.
    pub git_sha: String,
}

/// Prometheus text-format scrape response payload.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, ironic::OpenApiSchema)]
pub struct MetricsScrapeResponse {
    /// Raw Prometheus metrics formatted text.
    pub text: String,
}

/// Generic paginated page wrapper for Ironic paginated responses.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct PaginatedPageResponse {
    /// Items in current page.
    pub items: Vec<serde_json::Value>,
    /// Total count of items matching the query.
    pub total: u64,
    /// Current query offset.
    pub offset: u64,
    /// Maximum items per page requested or clamped.
    pub limit: u64,
}

impl ironic::OpenApiSchema for PaginatedPageResponse {
    fn openapi_schema() -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "title": "PaginatedPageResponse",
            "description": "Generic paginated response payload",
            "properties": {
                "items": {
                    "type": "array",
                    "description": "Page items",
                    "items": { "type": "object" }
                },
                "total": { "type": "integer", "format": "int64" },
                "offset": { "type": "integer", "format": "int64" },
                "limit": { "type": "integer", "format": "int64" }
            },
            "required": ["items", "total", "offset", "limit"]
        })
    }
}

/// Enriches raw OpenAPI 3.1 specification with metadata, tags, and response schemas
/// for built-in health and version routes.
pub fn enrich_openapi_spec(spec: &mut serde_json::Value) {
    let Some(paths) = spec
        .get_mut("paths")
        .and_then(serde_json::Value::as_object_mut)
    else {
        return;
    };

    if let Some(get_op) = paths
        .get_mut("/health")
        .and_then(serde_json::Value::as_object_mut)
        .and_then(|h| h.get_mut("get"))
        .and_then(serde_json::Value::as_object_mut)
    {
        get_op.insert("tags".into(), serde_json::json!(["System & Health"]));
        get_op.insert(
            "summary".into(),
            serde_json::json!("Composite health check"),
        );
        get_op.insert(
            "description".into(),
            serde_json::json!("Aggregates all registered health indicators (PostgreSQL, Valkey, NATS). Returns 200 when healthy, 207 when degraded, 503 when unhealthy."),
        );
        get_op.insert(
            "responses".into(),
            serde_json::json!({
                "200": {
                    "description": "All system components healthy",
                    "content": { "application/json": { "schema": { "$ref": "#/components/schemas/CompositeHealthResponse" } } }
                },
                "207": {
                    "description": "One or more non-critical components degraded",
                    "content": { "application/json": { "schema": { "$ref": "#/components/schemas/CompositeHealthResponse" } } }
                },
                "503": {
                    "description": "Critical infrastructure dependencies unhealthy",
                    "content": { "application/json": { "schema": { "$ref": "#/components/schemas/CompositeHealthResponse" } } }
                }
            }),
        );
    }

    if let Some(get_op) = paths
        .get_mut("/health/live")
        .and_then(serde_json::Value::as_object_mut)
        .and_then(|hl| hl.get_mut("get"))
        .and_then(serde_json::Value::as_object_mut)
    {
        get_op.insert("tags".into(), serde_json::json!(["System & Health"]));
        get_op.insert("summary".into(), serde_json::json!("Liveness probe"));
        get_op.insert(
            "description".into(),
            serde_json::json!(
                "Liveness probe indicating the server process is alive and responsive."
            ),
        );
        get_op.insert(
            "responses".into(),
            serde_json::json!({
                "200": {
                    "description": "Process is alive",
                    "content": { "application/json": { "schema": { "$ref": "#/components/schemas/LivenessResponse" } } }
                }
            }),
        );
    }

    if let Some(get_op) = paths
        .get_mut("/health/ready")
        .and_then(serde_json::Value::as_object_mut)
        .and_then(|hr| hr.get_mut("get"))
        .and_then(serde_json::Value::as_object_mut)
    {
        get_op.insert("tags".into(), serde_json::json!(["System & Health"]));
        get_op.insert("summary".into(), serde_json::json!("Readiness probe"));
        get_op.insert(
            "description".into(),
            serde_json::json!(
                "Readiness probe verifying database, cache, and message bus dependencies."
            ),
        );
        get_op.insert(
            "responses".into(),
            serde_json::json!({
                "200": {
                    "description": "All dependencies ready to serve traffic",
                    "content": { "application/json": { "schema": { "$ref": "#/components/schemas/CompositeHealthResponse" } } }
                },
                "503": {
                    "description": "One or more critical dependencies unavailable",
                    "content": { "application/json": { "schema": { "$ref": "#/components/schemas/CompositeHealthResponse" } } }
                }
            }),
        );
    }

    if let Some(get_op) = paths
        .get_mut("/version")
        .and_then(serde_json::Value::as_object_mut)
        .and_then(|v| v.get_mut("get"))
        .and_then(serde_json::Value::as_object_mut)
    {
        get_op.insert("tags".into(), serde_json::json!(["System & Health"]));
        get_op.insert(
            "summary".into(),
            serde_json::json!("System build and version information"),
        );
        get_op.insert(
            "description".into(),
            serde_json::json!("Returns current version, git revision SHA, and build metadata."),
        );
        get_op.insert(
            "responses".into(),
            serde_json::json!({
                "200": {
                    "description": "Build and runtime version metadata",
                    "content": { "application/json": { "schema": { "$ref": "#/components/schemas/SystemVersionResponse" } } }
                }
            }),
        );
    }

    if let Some(get_op) = paths
        .get_mut("/metrics")
        .and_then(serde_json::Value::as_object_mut)
        .and_then(|v| v.get_mut("get"))
        .and_then(serde_json::Value::as_object_mut)
    {
        get_op.insert("tags".into(), serde_json::json!(["System & Health"]));
        get_op.insert("summary".into(), serde_json::json!("Prometheus metrics"));
        get_op.insert(
            "description".into(),
            serde_json::json!("Returns framework, domain, and runtime Prometheus metrics."),
        );
        get_op.insert(
            "responses".into(),
            serde_json::json!({
                "200": {
                    "description": "Scraped Prometheus metrics payload",
                    "content": { "application/json": { "schema": { "$ref": "#/components/schemas/MetricsScrapeResponse" } } }
                }
            }),
        );
    }
}
