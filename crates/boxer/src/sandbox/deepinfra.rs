//! DeepInfra Kata microVM sandbox runtime driver implementing [`SandboxDriver`].

use std::{path::PathBuf, time::Duration};

use agents_protocol::{SandboxProfileError, TaskStreamEvent};
use tokio::sync::{mpsc, watch};

use crate::{
    error::BoxerError,
    sandbox::{BoxFuture, SandboxDriver, SandboxExecOutput, SandboxExecRequest},
};

/// DeepInfra microVM runtime driver running isolated Kata microVM containers.
#[derive(Clone)]
pub struct DeepInfraSandboxRuntimeDriver {
    endpoint: String,
    token: Option<String>,
    client: reqwest::Client,
}

impl DeepInfraSandboxRuntimeDriver {
    #[must_use]
    pub fn new(token: Option<String>) -> Self {
        Self {
            endpoint: "https://api.deepinfra.com".to_owned(),
            token,
            client: reqwest::Client::new(),
        }
    }

    #[must_use]
    pub fn with_endpoint(endpoint: String, token: Option<String>) -> Self {
        Self {
            endpoint: endpoint.trim_end_matches('/').to_owned(),
            token,
            client: reqwest::Client::new(),
        }
    }
}

impl SandboxDriver for DeepInfraSandboxRuntimeDriver {
    fn execute<'a>(
        &'a self,
        req: SandboxExecRequest,
        stream_tx: Option<mpsc::Sender<TaskStreamEvent>>,
        mut cancel_rx: watch::Receiver<bool>,
    ) -> BoxFuture<'a, Result<SandboxExecOutput, BoxerError>> {
        Box::pin(async move {
            let token = self.token.as_deref().ok_or(BoxerError::SandboxProfile(
                SandboxProfileError::ModeRuntimeMismatch,
            ))?;

            if *cancel_rx.borrow() {
                return Ok(SandboxExecOutput {
                    exit_code: 130,
                    stdout: String::new(),
                    stderr: "cancelled before provisioning".into(),
                    timed_out: false,
                    cancelled: true,
                    working_dir: PathBuf::from("/workspace"),
                });
            }

            // Create microVM sandbox
            let create_url = format!("{}/v1/sandboxes", self.endpoint);
            let timeout_secs = req.exec.timeout_seconds.unwrap_or(300);
            let create_payload = serde_json::json!({
                "plan": "small",
                "timeout_seconds": timeout_secs,
                "tags": { "task_id": req.task_id }
            });

            let create_res = self
                .client
                .post(&create_url)
                .bearer_auth(token)
                .json(&create_payload)
                .send()
                .await
                .map_err(|e| {
                    BoxerError::IsolationViolation(format!("deepinfra sandbox create network: {e}"))
                })?;

            let sandbox_id = if create_res.status().is_success() {
                let body: serde_json::Value = create_res.json().await.unwrap_or_default();
                body.get("id")
                    .and_then(|v| v.as_str())
                    .unwrap_or("sb-ephemeral")
                    .to_owned()
            } else {
                format!("sb-mock-{}", req.task_id)
            };

            // Execute command inside microVM
            let exec_url = format!("{}/v1/sandboxes/{}/exec", self.endpoint, sandbox_id);
            let parts: Vec<&str> = req.command.split_whitespace().collect();
            let exec_payload = serde_json::json!({
                "command": if parts.is_empty() { vec!["sh"] } else { parts },
                "timeout_seconds": req.exec.timeout_seconds.unwrap_or(60),
            });

            let exec_timeout = req.exec.timeout_seconds.unwrap_or(60).max(1);
            let exec_fut = self
                .client
                .post(&exec_url)
                .bearer_auth(token)
                .json(&exec_payload)
                .timeout(Duration::from_secs(exec_timeout))
                .send();

            let (stdout, stderr, exit_code, cancelled, timed_out) = tokio::select! {
                res = exec_fut => match res {
                    Ok(resp) if resp.status().is_success() => {
                        let data: serde_json::Value = resp.json().await.unwrap_or_default();
                        let out = data.get("stdout").and_then(|v| v.as_str()).unwrap_or("").to_owned();
                        let err = data.get("stderr").and_then(|v| v.as_str()).unwrap_or("").to_owned();
                        let code = data.get("returncode").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
                        (out, err, code, false, false)
                    }
                    Ok(resp) => {
                        let txt = resp.text().await.unwrap_or_default();
                        (String::new(), format!("deepinfra exec error: {txt}"), 1, false, false)
                    }
                    Err(e) if e.is_timeout() => {
                        (String::new(), "command timed out".into(), 124, false, true)
                    }
                    Err(e) => {
                        (String::new(), format!("network error: {e}"), 1, false, false)
                    }
                },
                _ = cancel_rx.changed() => {
                    (String::new(), "execution cancelled by caller".into(), 130, true, false)
                }
            };

            if let Some(ref tx) = stream_tx {
                if !stdout.is_empty() {
                    let _ = tx
                        .send(TaskStreamEvent {
                            task_id: req.task_id.clone(),
                            seq: 1,
                            emitted_at_ms: crate::config::now_ms(),
                            chunk: stdout.clone(),
                            is_stderr: false,
                        })
                        .await;
                }
                if !stderr.is_empty() {
                    let _ = tx
                        .send(TaskStreamEvent {
                            task_id: req.task_id.clone(),
                            seq: 2,
                            emitted_at_ms: crate::config::now_ms(),
                            chunk: stderr.clone(),
                            is_stderr: true,
                        })
                        .await;
                }
            }

            // Terminate sandbox to release compute resources
            let term_url = format!("{}/v1/sandboxes/{}", self.endpoint, sandbox_id);
            let _ = self
                .client
                .delete(&term_url)
                .bearer_auth(token)
                .send()
                .await;

            Ok(SandboxExecOutput {
                exit_code,
                stdout,
                stderr,
                timed_out,
                cancelled,
                working_dir: PathBuf::from("/workspace"),
            })
        })
    }
}
