//! OpenAPI 3.1 contract specification for the Boxer HTTP control plane (`agents-boxer`).

use serde_json::{Value, json};

fn json_ref_response(description: &str, schema_name: &str) -> Value {
    json!({
        "description": description,
        "content": {
            "application/json": {
                "schema": { "$ref": format!("#/components/schemas/{schema_name}") }
            }
        }
    })
}

fn json_ref_body(schema_name: &str) -> Value {
    json!({
        "required": true,
        "content": {
            "application/json": {
                "schema": { "$ref": format!("#/components/schemas/{schema_name}") }
            }
        }
    })
}

fn box_id_param() -> Value {
    json!({
        "name": "id",
        "in": "path",
        "required": true,
        "description": "Provisioned sandbox identifier",
        "schema": { "type": "string" }
    })
}

fn problem_responses() -> Vec<(&'static str, &'static str)> {
    vec![
        ("401", "Missing cluster secret"),
        ("403", "Invalid cluster secret"),
    ]
}

fn build_paths() -> Value {
    let mut box_exec_responses = serde_json::Map::new();
    box_exec_responses.insert(
        "200".into(),
        json_ref_response("Execution completed", "ExecBoxResponse"),
    );
    box_exec_responses.insert(
        "404".into(),
        json_ref_response("Sandbox not found", "BoxerProblemDetails"),
    );
    box_exec_responses.insert(
        "500".into(),
        json_ref_response("Execution failure", "BoxerProblemDetails"),
    );
    for (code, desc) in problem_responses() {
        box_exec_responses.insert(code.into(), json_ref_response(desc, "BoxerProblemDetails"));
    }

    json!({
        "/health": {
            "get": {
                "tags": ["System & Health"],
                "summary": "Consolidated node health and capacity",
                "description": "Returns current Boxer node readiness and sandbox allocation counts.",
                "responses": {
                    "200": json_ref_response("Boxer health status", "BoxerHealthResponse")
                }
            }
        },
        "/health/live": {
            "get": {
                "tags": ["System & Health"],
                "summary": "Liveness probe",
                "description": "Verifies that the Boxer daemon process is active.",
                "responses": {
                    "200": json_ref_response("Daemon is alive", "LivenessResponse")
                }
            }
        },
        "/health/ready": {
            "get": {
                "tags": ["System & Health"],
                "summary": "Readiness probe",
                "description": "Returns current node capacity and active box counts.",
                "responses": {
                    "200": json_ref_response("Boxer node ready to accept sandboxes", "BoxerHealthResponse")
                }
            }
        },
        "/version": {
            "get": {
                "tags": ["System & Health"],
                "summary": "Build metadata and version",
                "description": "Returns the Boxer service name and package version.",
                "responses": {
                    "200": json_ref_response("Service version metadata", "VersionResponse")
                }
            }
        },
        "/v1/box": {
            "post": {
                "tags": ["Sandboxes"],
                "summary": "Provision a new sandbox",
                "description": "Allocates an isolated workspace directory and provisions container runtime resources.",
                "security": [{ "clusterSecretAuth": [] }],
                "requestBody": json_ref_body("ProvisionBoxRequest"),
                "responses": {
                    "201": json_ref_response("Sandbox provisioned successfully", "ProvisionBoxResponse"),
                    "401": json_ref_response("Missing cluster secret", "BoxerProblemDetails"),
                    "403": json_ref_response("Invalid cluster secret", "BoxerProblemDetails"),
                    "429": json_ref_response("Capacity exceeded", "BoxerProblemDetails"),
                    "500": json_ref_response("Provisioning failure", "BoxerProblemDetails")
                }
            }
        },
        "/v1/box/{id}/exec": {
            "post": {
                "tags": ["Sandboxes"],
                "summary": "Execute command in sandbox",
                "description": "Runs a command hermetically inside the sandbox with stdout/stderr capture and cancellation guards.",
                "security": [{ "clusterSecretAuth": [] }],
                "parameters": [box_id_param()],
                "requestBody": json_ref_body("ExecBoxRequest"),
                "responses": Value::Object(box_exec_responses)
            }
        },
        "/v1/box/{id}/status": {
            "get": {
                "tags": ["Sandboxes"],
                "summary": "Get sandbox status and metrics",
                "security": [{ "clusterSecretAuth": [] }],
                "parameters": [box_id_param()],
                "responses": {
                    "200": json_ref_response("Current sandbox runtime metrics", "BoxStatusResponse"),
                    "401": json_ref_response("Missing cluster secret", "BoxerProblemDetails"),
                    "403": json_ref_response("Invalid cluster secret", "BoxerProblemDetails"),
                    "404": json_ref_response("Sandbox not found", "BoxerProblemDetails")
                }
            }
        },
        "/v1/box/{id}/files": {
            "get": {
                "tags": ["Sandboxes"],
                "summary": "Read file from sandbox VFS",
                "security": [{ "clusterSecretAuth": [] }],
                "parameters": [
                    box_id_param(),
                    {
                        "name": "path",
                        "in": "query",
                        "required": true,
                        "description": "Relative file path inside sandbox workspace",
                        "schema": { "type": "string" }
                    }
                ],
                "responses": {
                    "200": json_ref_response("File contents read successfully", "ReadFileResponse"),
                    "400": json_ref_response("Missing path or traversal attempt blocked", "BoxerProblemDetails"),
                    "401": json_ref_response("Missing cluster secret", "BoxerProblemDetails"),
                    "403": json_ref_response("Invalid cluster secret", "BoxerProblemDetails"),
                    "404": json_ref_response("File not found", "BoxerProblemDetails")
                }
            },
            "post": {
                "tags": ["Sandboxes"],
                "summary": "Write file to sandbox VFS",
                "security": [{ "clusterSecretAuth": [] }],
                "parameters": [box_id_param()],
                "requestBody": json_ref_body("WriteFileRequest"),
                "responses": {
                    "200": json_ref_response("File written successfully", "WriteFileResponse"),
                    "400": json_ref_response("Path traversal attempt blocked", "BoxerProblemDetails"),
                    "401": json_ref_response("Missing cluster secret", "BoxerProblemDetails"),
                    "403": json_ref_response("Invalid cluster secret", "BoxerProblemDetails"),
                    "404": json_ref_response("Sandbox not found", "BoxerProblemDetails"),
                    "500": json_ref_response("Write failure", "BoxerProblemDetails")
                }
            }
        },
        "/v1/box/{id}": {
            "delete": {
                "tags": ["Sandboxes"],
                "summary": "Teardown and purge sandbox",
                "security": [{ "clusterSecretAuth": [] }],
                "parameters": [box_id_param()],
                "responses": {
                    "200": json_ref_response("Sandbox teardown complete", "TeardownBoxResponse"),
                    "401": json_ref_response("Missing cluster secret", "BoxerProblemDetails"),
                    "403": json_ref_response("Invalid cluster secret", "BoxerProblemDetails"),
                    "404": json_ref_response("Sandbox not found", "BoxerProblemDetails")
                }
            }
        }
    })
}

fn build_schemas() -> Value {
    json!({
        "ProvisionBoxRequest": {
            "type": "object",
            "required": ["sessionId", "taskId"],
            "properties": {
                "sessionId": { "type": "string" },
                "taskId": { "type": "string" },
                "profile": { "type": "object", "nullable": true },
                "env": { "type": "object", "additionalProperties": { "type": "string" } }
            }
        },
        "ProvisionBoxResponse": {
            "type": "object",
            "required": ["boxId", "status", "workingDir", "createdAt"],
            "properties": {
                "boxId": { "type": "string" },
                "status": { "type": "string" },
                "workingDir": { "type": "string" },
                "createdAt": { "type": "integer", "format": "uint64" }
            }
        },
        "ExecBoxRequest": {
            "type": "object",
            "required": ["command"],
            "properties": {
                "command": { "type": "string" },
                "timeoutMs": { "type": "integer", "format": "uint64", "nullable": true }
            }
        },
        "ExecBoxResponse": {
            "type": "object",
            "required": ["exitCode", "stdout", "stderr", "timedOut", "cancelled", "durationMs"],
            "properties": {
                "exitCode": { "type": "integer", "format": "int32" },
                "stdout": { "type": "string" },
                "stderr": { "type": "string" },
                "timedOut": { "type": "boolean" },
                "cancelled": { "type": "boolean" },
                "durationMs": { "type": "integer", "format": "uint64" }
            }
        },
        "BoxStatusResponse": {
            "type": "object",
            "required": ["boxId", "status", "memoryBytes", "activeTasks", "uptimeSecs"],
            "properties": {
                "boxId": { "type": "string" },
                "status": { "type": "string" },
                "memoryBytes": { "type": "integer", "format": "uint64" },
                "activeTasks": { "type": "integer", "format": "uint32" },
                "uptimeSecs": { "type": "integer", "format": "uint64" }
            }
        },
        "WriteFileRequest": {
            "type": "object",
            "required": ["path", "content"],
            "properties": {
                "path": { "type": "string" },
                "content": { "type": "string" }
            }
        },
        "WriteFileResponse": {
            "type": "object",
            "required": ["status"],
            "properties": {
                "status": { "type": "string" }
            }
        },
        "ReadFileResponse": {
            "type": "object",
            "required": ["path", "content", "size"],
            "properties": {
                "path": { "type": "string" },
                "content": { "type": "string" },
                "size": { "type": "integer", "format": "uint64" }
            }
        },
        "TeardownBoxResponse": {
            "type": "object",
            "required": ["boxId", "status"],
            "properties": {
                "boxId": { "type": "string" },
                "status": { "type": "string" }
            }
        },
        "BoxerHealthResponse": {
            "type": "object",
            "required": ["status", "nodeId", "activeBoxes", "maxConcurrentBoxes"],
            "properties": {
                "status": { "type": "string" },
                "nodeId": { "type": "string" },
                "activeBoxes": { "type": "integer", "format": "uint" },
                "maxConcurrentBoxes": { "type": "integer", "format": "uint" }
            }
        },
        "LivenessResponse": {
            "type": "object",
            "required": ["status"],
            "properties": {
                "status": { "type": "string" }
            }
        },
        "VersionResponse": {
            "type": "object",
            "required": ["service", "version"],
            "properties": {
                "service": { "type": "string" },
                "version": { "type": "string" }
            }
        },
        "BoxerProblemDetails": {
            "type": "object",
            "required": ["type", "title", "status", "detail"],
            "properties": {
                "type": { "type": "string" },
                "title": { "type": "string" },
                "status": { "type": "integer", "format": "uint16" },
                "detail": { "type": "string" }
            }
        }
    })
}

/// Builds Boxer's OpenAPI 3.1 document with full typed request and response schemas.
#[must_use]
pub fn boxer_openapi_spec() -> Value {
    json!({
        "openapi": "3.1.0",
        "info": {
            "title": "agents-boxer API",
            "version": env!("CARGO_PKG_VERSION"),
            "description": "Internal sandbox supervisor and execution daemon control plane"
        },
        "tags": [
            { "name": "System & Health", "description": "Liveness, readiness, and version probes" },
            { "name": "Sandboxes", "description": "Confined sandbox provisioning, execution, and VFS file operations" }
        ],
        "paths": build_paths(),
        "components": {
            "securitySchemes": {
                "clusterSecretAuth": {
                    "type": "http",
                    "scheme": "bearer",
                    "description": "Boxer cluster secret passed via Authorization Bearer header or X-Boxer-Secret"
                }
            },
            "schemas": build_schemas()
        }
    })
}
