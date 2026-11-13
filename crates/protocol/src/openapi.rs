//! OpenAPI 3.1 schema definitions for shared protocol types and DTOs.

use ironic::OpenApiSchema;
use serde_json::{Value, json};

use crate::{
    ApiKeyMetadata, ApiKeyScope, BoxerNodeAnnouncement, CursorPage, EventEnvelope, ExecOptions,
    FencingToken, FollowUpMessage, InteractionMode, IsolationKind, NodeStatus, PermissionMask,
    RuntimeKind, SandboxProfile, SetupMode, SteeringMessage, StopReason, TaskLease, TaskResult,
    TaskState, TokenPair, TraefikPreviewRoute, TriggerKind, WorkflowDag, WorkflowEdge,
    WorkflowNode,
};

impl OpenApiSchema for ApiKeyScope {
    fn openapi_schema() -> Value {
        json!({
            "type": "string",
            "title": "ApiKeyScope",
            "enum": ["read", "write", "exec", "admin"],
            "example": "read"
        })
    }
}

impl OpenApiSchema for TokenPair {
    fn openapi_schema() -> Value {
        json!({
            "type": "object",
            "title": "TokenPair",
            "required": ["accessToken", "refreshToken", "tokenType", "expiresIn"],
            "properties": {
                "accessToken": { "type": "string", "example": "eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9..." },
                "refreshToken": { "type": "string", "example": "rf_9b8a7c6d5e4f3a2b1c0d" },
                "tokenType": { "type": "string", "example": "Bearer" },
                "expiresIn": { "type": "integer", "example": 3600 }
            }
        })
    }
}

impl OpenApiSchema for ApiKeyMetadata {
    fn openapi_schema() -> Value {
        json!({
            "type": "object",
            "title": "ApiKeyMetadata",
            "required": ["id", "userId", "label", "prefix", "scopes", "createdAt"],
            "properties": {
                "id": { "type": "string", "example": "ak_01j7xyz123abc456" },
                "userId": { "type": "string", "example": "usr_01j7abc987def654" },
                "label": { "type": "string", "example": "Production CLI Worker" },
                "prefix": { "type": "string", "example": "ag_ak_live" },
                "scopes": { "type": "array", "items": ApiKeyScope::openapi_schema() },
                "createdAt": { "type": "integer", "example": 1731513600 },
                "expiresAt": { "type": "integer", "nullable": true, "example": 1763049600 },
                "revokedAt": { "type": "integer", "nullable": true }
            }
        })
    }
}

impl OpenApiSchema for FencingToken {
    fn openapi_schema() -> Value {
        json!({
            "type": "integer",
            "title": "FencingToken",
            "minimum": 1,
            "example": 42
        })
    }
}

impl OpenApiSchema for TaskState {
    fn openapi_schema() -> Value {
        json!({
            "type": "string",
            "title": "TaskState",
            "enum": ["queued", "leased", "running", "blocked", "done", "failed", "cancelled"],
            "example": "running"
        })
    }
}

impl OpenApiSchema for InteractionMode {
    fn openapi_schema() -> Value {
        json!({
            "type": "string",
            "title": "InteractionMode",
            "enum": ["plan", "act"],
            "example": "act"
        })
    }
}

impl OpenApiSchema for StopReason {
    fn openapi_schema() -> Value {
        json!({
            "type": "string",
            "title": "StopReason",
            "enum": ["end_turn", "tool_use", "length", "aborted", "error"],
            "example": "end_turn"
        })
    }
}

impl OpenApiSchema for SetupMode {
    fn openapi_schema() -> Value {
        json!({
            "type": "string",
            "title": "SetupMode",
            "enum": ["local", "remote"],
            "example": "local"
        })
    }
}

impl OpenApiSchema for RuntimeKind {
    fn openapi_schema() -> Value {
        json!({
            "type": "string",
            "title": "RuntimeKind",
            "enum": ["host", "docker", "remote_vps", "upstash_box", "daytona", "deepinfra"],
            "example": "host"
        })
    }
}

impl OpenApiSchema for IsolationKind {
    fn openapi_schema() -> Value {
        json!({
            "type": "string",
            "title": "IsolationKind",
            "enum": ["none", "native", "openshell"],
            "example": "native"
        })
    }
}

impl OpenApiSchema for SandboxProfile {
    fn openapi_schema() -> Value {
        json!({
            "type": "object",
            "title": "SandboxProfile",
            "required": ["mode", "runtime", "isolation"],
            "properties": {
                "mode": SetupMode::openapi_schema(),
                "runtime": RuntimeKind::openapi_schema(),
                "isolation": IsolationKind::openapi_schema(),
                "allowHostNative": { "type": "boolean", "default": false },
                "gatewayUrl": { "type": "string", "nullable": true }
            }
        })
    }
}

impl OpenApiSchema for ExecOptions {
    fn openapi_schema() -> Value {
        json!({
            "type": "object",
            "title": "ExecOptions",
            "properties": {
                "workingDir": { "type": "string", "nullable": true, "example": "/workspace" },
                "timeoutSeconds": { "type": "integer", "nullable": true, "example": 60 },
                "maxMemoryMb": { "type": "integer", "nullable": true, "example": 512 },
                "maxCpus": { "type": "integer", "nullable": true, "example": 2 },
                "envAllowlist": { "type": "array", "items": { "type": "string" }, "example": ["PATH", "HOME"] },
                "networkEgress": { "type": "boolean", "default": false }
            }
        })
    }
}

impl OpenApiSchema for TaskLease {
    fn openapi_schema() -> Value {
        json!({
            "type": "object",
            "title": "TaskLease",
            "required": ["taskId", "sessionId", "boxerNodeId", "fencingToken", "acquiredAtMs", "expiresAtMs"],
            "properties": {
                "taskId": { "type": "string", "example": "tsk_01j7task001" },
                "sessionId": { "type": "string", "example": "ses_01j7sess001" },
                "boxerNodeId": { "type": "string", "example": "boxer-node-vlan-2" },
                "fencingToken": FencingToken::openapi_schema(),
                "acquiredAtMs": { "type": "integer", "example": 1731513600000_u64 },
                "expiresAtMs": { "type": "integer", "example": 1731513630000_u64 }
            }
        })
    }
}

impl OpenApiSchema for TaskResult {
    fn openapi_schema() -> Value {
        json!({
            "type": "object",
            "title": "TaskResult",
            "required": ["taskId", "exitCode", "stopReason", "completedAt"],
            "properties": {
                "taskId": { "type": "string", "example": "tsk_01j7task001" },
                "fencingToken": { "type": "integer", "nullable": true, "example": 42 },
                "exitCode": { "type": "integer", "example": 0 },
                "stopReason": StopReason::openapi_schema(),
                "outputSummary": { "type": "string", "nullable": true, "example": "Generated index.html successfully" },
                "completedAt": { "type": "integer", "example": 1731513605 }
            }
        })
    }
}

impl OpenApiSchema for SteeringMessage {
    fn openapi_schema() -> Value {
        json!({
            "type": "object",
            "title": "SteeringMessage",
            "required": ["messageId", "sessionId", "taskId", "content", "emittedAtMs"],
            "properties": {
                "messageId": { "type": "string", "example": "msg_01j7steer" },
                "sessionId": { "type": "string", "example": "ses_01j7sess001" },
                "taskId": { "type": "string", "example": "tsk_01j7task001" },
                "content": { "type": "string", "example": "Focus on responsive CSS first" },
                "emittedAtMs": { "type": "integer", "example": 1731513602000_u64 }
            }
        })
    }
}

impl OpenApiSchema for FollowUpMessage {
    fn openapi_schema() -> Value {
        json!({
            "type": "object",
            "title": "FollowUpMessage",
            "required": ["messageId", "sessionId", "content", "queuedAtMs"],
            "properties": {
                "messageId": { "type": "string", "example": "flw_01j7follow" },
                "sessionId": { "type": "string", "example": "ses_01j7sess001" },
                "content": { "type": "string", "example": "Now run validation suite on the generated output" },
                "queuedAtMs": { "type": "integer", "example": 1731513605000_u64 }
            }
        })
    }
}

impl OpenApiSchema for BoxerNodeAnnouncement {
    fn openapi_schema() -> Value {
        json!({
            "type": "object",
            "title": "BoxerNodeAnnouncement",
            "required": ["nodeId", "privateIp", "boxerPort", "totalSlots", "availableSlots", "loadAverage", "registeredAtMs"],
            "properties": {
                "nodeId": { "type": "string", "example": "boxer-vlan-worker-01" },
                "privateIp": { "type": "string", "example": "10.0.0.12" },
                "boxerPort": { "type": "integer", "example": 9091 },
                "totalSlots": { "type": "integer", "example": 16 },
                "availableSlots": { "type": "integer", "example": 14 },
                "loadAverage": { "type": "number", "example": 0.45 },
                "registeredAtMs": { "type": "integer", "example": 1731513600000_u64 }
            }
        })
    }
}

impl OpenApiSchema for EventEnvelope {
    fn openapi_schema() -> Value {
        json!({
            "type": "object",
            "title": "EventEnvelope",
            "required": ["seq", "sessionId", "emittedAtMs", "payload"],
            "properties": {
                "seq": { "type": "integer", "example": 1 },
                "sessionId": { "type": "string", "example": "ses_01j7sess001" },
                "taskId": { "type": "string", "nullable": true, "example": "tsk_01j7task001" },
                "emittedAtMs": { "type": "integer", "example": 1731513601000_u64 },
                "payload": { "type": "object" }
            }
        })
    }
}

impl<T: OpenApiSchema> OpenApiSchema for CursorPage<T> {
    fn openapi_schema() -> Value {
        json!({
            "type": "object",
            "title": "CursorPage",
            "required": ["items", "totalCount", "hasMore"],
            "properties": {
                "items": { "type": "array", "items": T::openapi_schema() },
                "nextCursor": { "type": "string", "nullable": true, "example": "cur_01j7xyz" },
                "totalCount": { "type": "integer", "example": 42 },
                "hasMore": { "type": "boolean", "example": false }
            }
        })
    }
}

impl OpenApiSchema for NodeStatus {
    fn openapi_schema() -> Value {
        json!({
            "type": "string",
            "title": "NodeStatus",
            "enum": ["online", "draining", "offline"],
            "example": "online"
        })
    }
}

impl OpenApiSchema for TraefikPreviewRoute {
    fn openapi_schema() -> Value {
        json!({
            "type": "object",
            "title": "TraefikPreviewRoute",
            "required": ["routeId", "subdomain", "targetIp", "targetPort", "active"],
            "properties": {
                "routeId": { "type": "string", "example": "prev_01j7xyz" },
                "subdomain": { "type": "string", "example": "test-box-preview" },
                "targetIp": { "type": "string", "example": "10.0.0.15" },
                "targetPort": { "type": "integer", "example": 8080 },
                "active": { "type": "boolean", "example": true }
            }
        })
    }
}

impl OpenApiSchema for PermissionMask {
    fn openapi_schema() -> Value {
        json!({
            "type": "object",
            "title": "PermissionMask",
            "properties": {
                "allowFsWrite": { "type": "boolean", "default": false },
                "allowNetworkEgress": { "type": "boolean", "default": false },
                "allowSubagentSpawn": { "type": "boolean", "default": false },
                "allowTerminalExec": { "type": "boolean", "default": false }
            }
        })
    }
}

impl OpenApiSchema for TriggerKind {
    fn openapi_schema() -> Value {
        json!({
            "type": "string",
            "title": "TriggerKind",
            "enum": ["cron", "interval", "webhook"],
            "example": "cron"
        })
    }
}

impl OpenApiSchema for WorkflowNode {
    fn openapi_schema() -> Value {
        json!({
            "type": "object",
            "title": "WorkflowNode",
            "required": ["nodeId", "kind", "config"],
            "properties": {
                "nodeId": { "type": "string", "example": "node_1" },
                "kind": { "type": "string", "example": "agent" },
                "config": { "type": "object", "example": { "prompt": "Execute analysis" } }
            }
        })
    }
}

impl OpenApiSchema for WorkflowEdge {
    fn openapi_schema() -> Value {
        json!({
            "type": "object",
            "title": "WorkflowEdge",
            "required": ["fromNodeId", "toNodeId"],
            "properties": {
                "fromNodeId": { "type": "string", "example": "node_1" },
                "toNodeId": { "type": "string", "example": "node_2" },
                "condition": { "type": "string", "nullable": true }
            }
        })
    }
}

impl OpenApiSchema for WorkflowDag {
    fn openapi_schema() -> Value {
        json!({
            "type": "object",
            "title": "WorkflowDag",
            "required": ["workflowId", "name", "nodes", "edges"],
            "properties": {
                "workflowId": { "type": "string", "example": "wf_01j7dag001" },
                "name": { "type": "string", "example": "Analysis Pipeline" },
                "nodes": { "type": "array", "items": WorkflowNode::openapi_schema() },
                "edges": { "type": "array", "items": WorkflowEdge::openapi_schema() },
                "webhookSlug": { "type": "string", "nullable": true, "example": "hook-analysis" },
                "renderSlug": { "type": "string", "nullable": true, "example": "view-analysis" },
                "approved": { "type": "boolean", "nullable": true, "example": true },
                "createdByAgent": { "type": "boolean", "nullable": true, "example": false }
            }
        })
    }
}
