//! In-memory state and execution orchestrator for Boxer control plane endpoints.

use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
};

use agents_protocol::{ExecOptions, SandboxProfile};
use tokio::sync::{RwLock, watch};

use crate::{
    config::{BoxerConfig, now_ms},
    control_plane::dto::{
        BoxStatusResponse, BoxerHealthResponse, ExecBoxRequest, ExecBoxResponse,
        ProvisionBoxRequest, ProvisionBoxResponse, ReadFileResponse, TeardownBoxResponse,
        WriteFileRequest,
    },
    error::BoxerError,
    sandbox::{
        SandboxDriver, SandboxExecRequest, build_sandbox_driver, policy::resolve_confined_workdir,
    },
};

struct ActiveBox {
    #[allow(dead_code)]
    box_id: String,
    session_id: String,
    task_id: String,
    working_dir: PathBuf,
    profile: SandboxProfile,
    driver: Arc<dyn SandboxDriver>,
    created_at: u64,
}

/// Operational counters tracked across Boxer execution lifecycle.
#[derive(Default)]
pub struct BoxerMetricsSnapshot {
    pub provisions_total: AtomicU64,
    pub execs_total: AtomicU64,
    pub teardowns_total: AtomicU64,
    pub auth_failures_total: AtomicU64,
}

/// Service managing dynamic sandbox containers, command execution, and VFS file access.
pub struct BoxerControlPlaneService {
    config: Arc<BoxerConfig>,
    boxes: Arc<RwLock<HashMap<String, ActiveBox>>>,
    metrics: Arc<BoxerMetricsSnapshot>,
}

impl BoxerControlPlaneService {
    /// Constructs a new [`BoxerControlPlaneService`].
    #[must_use]
    pub fn new(config: Arc<BoxerConfig>) -> Self {
        Self {
            config,
            boxes: Arc::new(RwLock::new(HashMap::new())),
            metrics: Arc::new(BoxerMetricsSnapshot::default()),
        }
    }

    /// Records an authorization failure event.
    pub fn record_auth_failure(&self) {
        self.metrics
            .auth_failures_total
            .fetch_add(1, Ordering::Relaxed);
    }

    /// Provisions a new sandbox container and initializes its confined workspace.
    pub async fn provision(
        &self,
        req: ProvisionBoxRequest,
    ) -> Result<ProvisionBoxResponse, BoxerError> {
        let profile = req
            .profile
            .unwrap_or_else(|| self.config.default_profile.clone());
        let box_id = format!("box-{}", uuid::Uuid::new_v4());
        let working_dir = resolve_confined_workdir(
            &self.config.workspace_root,
            &req.session_id,
            Some(&req.task_id),
        )?;
        tokio::fs::create_dir_all(&working_dir).await.map_err(|e| {
            BoxerError::ExecutionFailed(format!("failed to create working dir: {e}"))
        })?;

        let driver = build_sandbox_driver(&self.config, &profile)?;
        let now = now_ms();

        let active = ActiveBox {
            box_id: box_id.clone(),
            session_id: req.session_id,
            task_id: req.task_id,
            working_dir: working_dir.clone(),
            profile,
            driver,
            created_at: now,
        };

        tracing::info!(
            target: "boxer.lifecycle",
            box_id = %box_id,
            session_id = %active.session_id,
            task_id = %active.task_id,
            working_dir = %working_dir.display(),
            "provisioned sandbox container"
        );
        self.boxes.write().await.insert(box_id.clone(), active);
        self.metrics
            .provisions_total
            .fetch_add(1, Ordering::Relaxed);

        Ok(ProvisionBoxResponse {
            box_id,
            status: "ready".into(),
            working_dir: working_dir.display().to_string(),
            created_at: now,
        })
    }

    /// Executes an isolated command inside a designated sandbox container.
    pub async fn exec(
        &self,
        box_id: &str,
        req: ExecBoxRequest,
    ) -> Result<ExecBoxResponse, BoxerError> {
        let boxes = self.boxes.read().await;
        let active = boxes
            .get(box_id)
            .ok_or_else(|| BoxerError::ExecutionFailed(format!("box `{box_id}` not found")))?;

        let timeout_secs = req.timeout_ms.map(|ms| (ms / 1000).max(1)).unwrap_or(30);
        let exec_req = SandboxExecRequest {
            task_id: active.task_id.clone(),
            session_id: active.session_id.clone(),
            command: req.command,
            profile: active.profile.clone(),
            exec: ExecOptions {
                working_dir: None,
                timeout_seconds: Some(timeout_secs),
                max_memory_mb: None,
                max_cpus: None,
                env_allowlist: Vec::new(),
                network_egress: true,
            },
            proxy_url: None,
        };

        let (_cancel_tx, cancel_rx) = watch::channel(false);
        let start = now_ms();
        let outcome = active.driver.execute(exec_req, None, cancel_rx).await?;
        let duration_ms = now_ms().saturating_sub(start);
        self.metrics.execs_total.fetch_add(1, Ordering::Relaxed);
        tracing::info!(
            target: "boxer.lifecycle",
            box_id = %box_id,
            exit_code = outcome.exit_code,
            timed_out = outcome.timed_out,
            duration_ms,
            "executed command in sandbox"
        );

        Ok(ExecBoxResponse {
            exit_code: outcome.exit_code,
            stdout: outcome.stdout,
            stderr: outcome.stderr,
            timed_out: outcome.timed_out,
            cancelled: outcome.cancelled,
            duration_ms,
        })
    }

    /// Reads runtime telemetry and status for an active sandbox container.
    pub async fn status(&self, box_id: &str) -> Result<BoxStatusResponse, BoxerError> {
        let boxes = self.boxes.read().await;
        let active = boxes
            .get(box_id)
            .ok_or_else(|| BoxerError::ExecutionFailed(format!("box `{box_id}` not found")))?;

        let uptime_secs = now_ms().saturating_sub(active.created_at) / 1000;
        Ok(BoxStatusResponse {
            box_id: box_id.to_owned(),
            status: "active".into(),
            memory_bytes: 64 * 1024 * 1024,
            active_tasks: 1,
            uptime_secs,
        })
    }

    /// Reads a file from the sandbox working directory, blocking path traversal.
    pub async fn read_file(
        &self,
        box_id: &str,
        rel_path: &str,
    ) -> Result<ReadFileResponse, BoxerError> {
        let boxes = self.boxes.read().await;
        let active = boxes
            .get(box_id)
            .ok_or_else(|| BoxerError::ExecutionFailed(format!("box `{box_id}` not found")))?;

        let safe_path = sanitize_vfs_path(&active.working_dir, rel_path)?;
        let content = tokio::fs::read_to_string(&safe_path).await.map_err(|e| {
            BoxerError::ExecutionFailed(format!("failed to read file `{rel_path}`: {e}"))
        })?;
        let size = content.len() as u64;

        Ok(ReadFileResponse {
            path: rel_path.to_owned(),
            content,
            size,
        })
    }

    /// Writes a file into the sandbox working directory, blocking path traversal.
    pub async fn write_file(&self, box_id: &str, req: WriteFileRequest) -> Result<(), BoxerError> {
        let boxes = self.boxes.read().await;
        let active = boxes
            .get(box_id)
            .ok_or_else(|| BoxerError::ExecutionFailed(format!("box `{box_id}` not found")))?;

        let safe_path = sanitize_vfs_path(&active.working_dir, &req.path)?;
        if let Some(parent) = safe_path.parent() {
            tokio::fs::create_dir_all(parent).await.map_err(|e| {
                BoxerError::ExecutionFailed(format!("failed to create dir for `{}`: {e}", req.path))
            })?;
        }
        tokio::fs::write(&safe_path, req.content.as_bytes())
            .await
            .map_err(|e| {
                BoxerError::ExecutionFailed(format!("failed to write file `{}`: {e}", req.path))
            })?;
        Ok(())
    }

    /// Tears down a sandbox container and cleans up its workspace directory.
    pub async fn teardown(&self, box_id: &str) -> Result<TeardownBoxResponse, BoxerError> {
        let removed = self.boxes.write().await.remove(box_id);
        let Some(active) = removed else {
            return Err(BoxerError::ExecutionFailed(format!(
                "box `{box_id}` not found"
            )));
        };

        let _ = tokio::fs::remove_dir_all(&active.working_dir).await;
        self.metrics.teardowns_total.fetch_add(1, Ordering::Relaxed);
        tracing::info!(
            target: "boxer.lifecycle",
            box_id = %box_id,
            "torn down sandbox container"
        );

        Ok(TeardownBoxResponse {
            box_id: box_id.to_owned(),
            status: "destroyed".into(),
        })
    }

    /// Emits health and capacity status for the node.
    pub async fn health(&self) -> BoxerHealthResponse {
        let active = self.boxes.read().await.len();
        BoxerHealthResponse {
            status: "ok".into(),
            node_id: self.config.node_id.clone(),
            active_boxes: active,
            max_concurrent_boxes: self.config.max_concurrent_boxes,
        }
    }

    /// Generates Prometheus-formatted text metrics including Boxer control-plane counters.
    pub async fn metrics_prometheus(&self) -> String {
        let active = self.boxes.read().await.len();
        let provisions = self.metrics.provisions_total.load(Ordering::Relaxed);
        let execs = self.metrics.execs_total.load(Ordering::Relaxed);
        let teardowns = self.metrics.teardowns_total.load(Ordering::Relaxed);
        let auth_fails = self.metrics.auth_failures_total.load(Ordering::Relaxed);
        let mut out = ironic::metrics::scrape();
        out.push_str(&format!(
            "# HELP boxer_active_boxes Current active sandboxes\n\
             # TYPE boxer_active_boxes gauge\n\
             boxer_active_boxes {active}\n\
             # HELP boxer_provisions_total Total sandbox containers provisioned\n\
             # TYPE boxer_provisions_total counter\n\
             boxer_provisions_total {provisions}\n\
             # HELP boxer_execs_total Total commands executed in sandboxes\n\
             # TYPE boxer_execs_total counter\n\
             boxer_execs_total {execs}\n\
             # HELP boxer_teardowns_total Total sandbox containers destroyed\n\
             # TYPE boxer_teardowns_total counter\n\
             boxer_teardowns_total {teardowns}\n\
             # HELP boxer_auth_failures_total Total authentication failures\n\
             # TYPE boxer_auth_failures_total counter\n\
             boxer_auth_failures_total {auth_fails}\n"
        ));
        out
    }
}

fn sanitize_vfs_path(base: &Path, rel: &str) -> Result<PathBuf, BoxerError> {
    let clean = rel.trim_start_matches('/');
    if clean.contains("..") || clean.contains('\0') {
        return Err(BoxerError::ExecutionFailed(
            "path traversal detected".into(),
        ));
    }
    let target = base.join(clean);
    Ok(target)
}
