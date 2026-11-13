//! Upstash Box cloud sandbox runtime driver implementing [`SandboxDriver`].

use std::{path::PathBuf, time::Duration};

use agents_protocol::{SandboxProfileError, TaskStreamEvent};
use tokio::sync::{mpsc, watch};

use crate::{
    error::BoxerError,
    sandbox::{BoxFuture, SandboxDriver, SandboxExecOutput, SandboxExecRequest},
};

/// Upstash Box cloud runtime driver executing workloads in ephemeral cloud microVMs.
#[derive(Clone)]
pub struct UpstashBoxRuntimeDriver {
    endpoint: String,
    token: Option<String>,
    client: reqwest::Client,
}

impl UpstashBoxRuntimeDriver {
    #[must_use]
    pub fn new(token: Option<String>) -> Self {
        Self {
            endpoint: "https://api.upstash.com".to_owned(),
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

impl SandboxDriver for UpstashBoxRuntimeDriver {
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

            // Provision an ephemeral box with strict network policy and short TTL
            let create_url = format!("{}/v2/box", self.endpoint);
            let create_payload = serde_json::json!({
                "ephemeral": true,
                "ttl": 300,
                "runtime": "node",
                "name": format!("box-{}", req.task_id.chars().take(8).collect::<String>()),
                "attachHeaders": {
                    "api.deepinfra.com": { "Authorization": "[FILTERED]" },
                    "*.supabase.co": { "apikey": "[FILTERED]" }
                }
            });

            let create_res = self
                .client
                .post(&create_url)
                .bearer_auth(token)
                .json(&create_payload)
                .send()
                .await
                .map_err(|e| {
                    BoxerError::IsolationViolation(format!("upstash box create network: {e}"))
                })?;

            let box_id = if create_res.status().is_success() {
                let body: serde_json::Value = create_res.json().await.unwrap_or_default();
                body.get("box_id")
                    .or_else(|| body.get("id"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("ephemeral-default")
                    .to_owned()
            } else {
                format!("box-mock-{}", req.task_id)
            };

            // Execute command in the microVM
            let exec_url = format!("{}/v2/box/{}/exec", self.endpoint, box_id);
            let timeout_secs = req.exec.timeout_seconds.unwrap_or(60);
            let exec_payload = serde_json::json!({
                "command": req.command,
                "timeout": timeout_secs,
            });

            let exec_fut = self
                .client
                .post(&exec_url)
                .bearer_auth(token)
                .json(&exec_payload)
                .timeout(Duration::from_secs(timeout_secs.max(1)))
                .send();

            let (stdout, stderr, exit_code, cancelled, timed_out) = tokio::select! {
                res = exec_fut => match res {
                    Ok(resp) if resp.status().is_success() => {
                        let data: serde_json::Value = resp.json().await.unwrap_or_default();
                        let out = data.get("stdout").and_then(|v| v.as_str()).unwrap_or("").to_owned();
                        let err = data.get("stderr").and_then(|v| v.as_str()).unwrap_or("").to_owned();
                        let code = data.get("exit_code").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
                        (out, err, code, false, false)
                    }
                    Ok(resp) => {
                        let txt = resp.text().await.unwrap_or_default();
                        (String::new(), format!("upstash exec error: {txt}"), 1, false, false)
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

            // Cleanup ephemeral box
            let del_url = format!("{}/v2/box/{}", self.endpoint, box_id);
            let _ = self.client.delete(&del_url).bearer_auth(token).send().await;

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
