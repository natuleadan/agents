//! Execution dispatcher for distinct workflow node kinds.

use agents_protocol::{ExecOptions, InteractionMode, TaskRequest, WorkflowNode};
use serde_json::{Value, json};

use crate::{
    infra::{error::ServerError, stream::BusClient},
    modules::workflows::entities::WorkflowRunEntity,
};

/// Dispatches execution of a single DAG node based on its kind.
///
/// # Errors
///
/// Returns [`ServerError`] if bus dispatch or underlying client setup fails.
pub async fn execute_single_node(
    bus: &BusClient,
    run: &WorkflowRunEntity,
    node: &WorkflowNode,
    _outputs: &serde_json::Map<String, Value>,
) -> Result<Value, ServerError> {
    match node.kind.as_str() {
        "http_request" => {
            let url = node
                .config
                .get("url")
                .and_then(Value::as_str)
                .unwrap_or("https://natuleadan.com");
            let method = node
                .config
                .get("method")
                .and_then(Value::as_str)
                .unwrap_or("GET");
            let client = reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(10))
                .build()
                .map_err(|e| ServerError::Internal(format!("http client: {e}")))?;
            let start = std::time::Instant::now();
            let resp = match method.to_uppercase().as_str() {
                "POST" => {
                    let body = node.config.get("body").unwrap_or(&json!({})).clone();
                    client.post(url).json(&body).send().await
                }
                _ => client.get(url).send().await,
            };
            let latency_ms = start.elapsed().as_millis() as u64;
            match resp {
                Ok(r) => {
                    let status = r.status().as_u16();
                    let success = r.status().is_success();
                    let text = r.text().await.unwrap_or_default();
                    Ok(json!({
                        "status": status,
                        "latencyMs": latency_ms,
                        "success": success,
                        "body": text,
                    }))
                }
                Err(e) => Ok(json!({
                    "status": 0,
                    "latencyMs": latency_ms,
                    "success": false,
                    "error": e.to_string(),
                })),
            }
        }
        "agent_step" => {
            let prompt = node
                .config
                .get("prompt")
                .and_then(Value::as_str)
                .unwrap_or("execute workflow step")
                .to_owned();
            let req = TaskRequest {
                task_id: format!("{}-{}", run.run_id, node.node_id),
                session_id: format!("wf-{}", run.workflow_id),
                prompt,
                timeout_seconds: Some(120),
                mode: InteractionMode::Act,
                fencing_token: None,
                idempotency_key: Some(format!("wf:{}:{}", run.run_id, node.node_id)),
                sandbox: None,
                exec: ExecOptions::default(),
            };
            bus.0.publish_task_request(&req).await?;
            Ok(json!({ "dispatched": true, "taskId": req.task_id }))
        }
        "render_html" | "html_output" => {
            let html = node
                .config
                .get("html")
                .and_then(Value::as_str)
                .unwrap_or("<p>Default render output</p>");
            Ok(json!({
                "html": html,
                "contentType": "text/html; charset=utf-8",
            }))
        }
        "condition" => {
            let result = node
                .config
                .get("branch")
                .cloned()
                .unwrap_or(Value::Bool(true));
            Ok(json!({ "branch": result }))
        }
        _ => Ok(json!({ "ok": true, "kind": node.kind })),
    }
}
