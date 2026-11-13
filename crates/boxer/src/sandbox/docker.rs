//! Hardened Docker container runtime driver supporting `None`, `Native`, and `Openshell`
//! isolation policies with read-only rootfs, dropped capabilities, cgroups, and `DropGuard`.

use std::{path::PathBuf, process::Stdio, sync::Arc, time::Duration};

use agents_protocol::{IsolationKind, TaskStreamEvent};
use tokio::{
    io::{AsyncBufReadExt, BufReader},
    process::Command,
    sync::{mpsc, watch},
};

use crate::{
    config::now_ms,
    error::BoxerError,
    leases::{CleanupDropGuard, SandboxResources},
    sandbox::{
        BoxFuture, SandboxDriver, SandboxExecOutput, SandboxExecRequest,
        policy::{build_sanitized_env, resolve_confined_workdir, validate_runtime_isolation},
    },
};

/// Deterministic container command specification built before execution (and testable without daemon).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DockerCommandSpec {
    pub container_name: String,
    pub image: String,
    pub args: Vec<String>,
    pub workdir: PathBuf,
}

/// Provisions isolated ephemeral Docker containers for agent sandbox execution.
#[derive(Debug, Clone)]
pub struct DockerRuntimeDriver {
    workspace_root: PathBuf,
    image: String,
}

impl DockerRuntimeDriver {
    #[must_use]
    pub fn new(workspace_root: PathBuf, image: impl Into<String>) -> Self {
        Self {
            workspace_root,
            image: image.into(),
        }
    }

    /// Builds hardened `docker run` arguments enforcing cgroup limits, `--cap-drop=ALL`,
    /// read-only rootfs, scrubbed env, and Openshell gateway policy.
    ///
    /// # Errors
    ///
    /// Returns [`BoxerError`] if profile or workspace validation fails.
    pub fn build_command_spec(
        &self,
        req: &SandboxExecRequest,
    ) -> Result<DockerCommandSpec, BoxerError> {
        validate_runtime_isolation(&req.profile, false)?;

        let workdir = resolve_confined_workdir(
            &self.workspace_root,
            &req.session_id,
            req.exec.working_dir.as_deref(),
        )?;

        let sanitized_task_id: String = req
            .task_id
            .chars()
            .filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
            .take(40)
            .collect();
        let container_name = format!("agents-box-{sanitized_task_id}");

        let mem_mb = req.exec.max_memory_mb.unwrap_or(512).max(64);
        let cpus = req.exec.max_cpus.unwrap_or(2).max(1);
        let mount_arg = format!("type=bind,src={},dst=/workspace", workdir.display());

        let mut args = vec![
            "run".to_owned(),
            "--rm".to_owned(),
            "--init".to_owned(),
            "--name".to_owned(),
            container_name.clone(),
            "--read-only".to_owned(),
            "--tmpfs".to_owned(),
            "/tmp:rw,noexec,nosuid,size=256m".to_owned(),
            "--cap-drop=ALL".to_owned(),
            "--security-opt=no-new-privileges:true".to_owned(),
            "--pids-limit=256".to_owned(),
            format!("--memory={mem_mb}m"),
            format!("--cpus={cpus}"),
            "--mount".to_owned(),
            mount_arg,
            "-w".to_owned(),
            "/workspace".to_owned(),
            "--label".to_owned(),
            format!("agents.isolation={}", req.profile.isolation.as_str()),
        ];

        if !req.exec.network_egress {
            args.push("--network=none".to_owned());
        } else {
            args.push("--network=agents-vlan".to_owned());
        }

        if req.profile.isolation == IsolationKind::Openshell
            && let Some(gateway) = &req.profile.gateway_url
        {
            args.push("--label".to_owned());
            args.push(format!("agents.openshell.gateway={gateway}"));
        }

        let env_map = build_sanitized_env(&req.exec, req.proxy_url.as_deref());
        for (k, v) in env_map {
            args.push("-e".to_owned());
            args.push(format!("{k}={v}"));
        }

        args.push(self.image.clone());
        args.push("sh".to_owned());
        args.push("-c".to_owned());
        args.push(req.command.clone());

        Ok(DockerCommandSpec {
            container_name,
            image: self.image.clone(),
            args,
            workdir,
        })
    }
}

impl SandboxDriver for DockerRuntimeDriver {
    fn execute<'a>(
        &'a self,
        req: SandboxExecRequest,
        stream_tx: Option<mpsc::Sender<TaskStreamEvent>>,
        mut cancel_rx: watch::Receiver<bool>,
    ) -> BoxFuture<'a, Result<SandboxExecOutput, BoxerError>> {
        Box::pin(async move {
            let spec = self.build_command_spec(&req)?;
            let timeout = Duration::from_secs(req.exec.timeout_seconds.unwrap_or(180).max(1));

            let mut resources = SandboxResources::new(&req.task_id);
            resources.container_name = Some(spec.container_name.clone());
            let guard = CleanupDropGuard::new(resources, |res| res.cleanup_sync());

            let mut cmd = Command::new("docker");
            cmd.args(&spec.args)
                .stdin(Stdio::null())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .kill_on_drop(true);

            let mut child = cmd.spawn().map_err(|err| {
                BoxerError::ExecutionFailed(format!("failed to invoke docker runtime: {err}"))
            })?;

            let stdout_pipe = child.stdout.take();
            let stderr_pipe = child.stderr.take();
            let seq = Arc::new(std::sync::atomic::AtomicU64::new(1));

            let out_handle = tokio::spawn(drain_docker_lines(
                req.task_id.clone(),
                stdout_pipe,
                false,
                stream_tx.clone(),
                Arc::clone(&seq),
            ));
            let err_handle = tokio::spawn(drain_docker_lines(
                req.task_id.clone(),
                stderr_pipe,
                true,
                stream_tx,
                seq,
            ));

            let mut timed_out = false;
            let mut cancelled = false;
            let code = tokio::select! {
                res = child.wait() => {
                    res.map_err(|err| BoxerError::ExecutionFailed(format!("docker wait failed: {err}")))?
                        .code()
                        .unwrap_or(-1)
                }
                _ = tokio::time::sleep(timeout) => {
                    timed_out = true;
                    let _ = Command::new("docker").args(["rm", "-f", &spec.container_name]).output().await;
                    -15
                }
                is_cancelled = async { cancel_rx.wait_for(|v| *v).await.is_ok() } => {
                    if is_cancelled {
                        cancelled = true;
                        let _ = Command::new("docker").args(["rm", "-f", &spec.container_name]).output().await;
                        -15
                    } else {
                        child.wait().await
                            .map_err(|err| BoxerError::ExecutionFailed(format!("docker wait failed: {err}")))?
                            .code()
                            .unwrap_or(-1)
                    }
                }
            };

            let stdout = out_handle.await.unwrap_or_default();
            let stderr = err_handle.await.unwrap_or_default();
            guard.run_now();

            Ok(SandboxExecOutput {
                exit_code: code,
                stdout,
                stderr,
                timed_out,
                cancelled,
                working_dir: spec.workdir,
            })
        })
    }
}

async fn drain_docker_lines<R: tokio::io::AsyncRead + Unpin>(
    task_id: String,
    reader: Option<R>,
    is_stderr: bool,
    tx: Option<mpsc::Sender<TaskStreamEvent>>,
    seq: Arc<std::sync::atomic::AtomicU64>,
) -> String {
    let Some(r) = reader else {
        return String::new();
    };
    let mut lines = BufReader::new(r).lines();
    let mut buf = String::new();
    while let Ok(Some(line)) = lines.next_line().await {
        if !buf.is_empty() {
            buf.push('\n');
        }
        buf.push_str(&line);
        if let Some(sender) = &tx {
            let _ = sender
                .send(TaskStreamEvent {
                    task_id: task_id.clone(),
                    seq: seq.fetch_add(1, std::sync::atomic::Ordering::SeqCst),
                    emitted_at_ms: now_ms(),
                    chunk: line,
                    is_stderr,
                })
                .await;
        }
    }
    buf
}
