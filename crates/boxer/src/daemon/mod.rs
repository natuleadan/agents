//! Boxer daemon orchestrating NATS JetStream queue consumption, fenced leases,
//! sandbox execution, telemetry emission, artifacts, and ephemeral previews.

pub mod bus;

use std::{
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::Duration,
};

use agents_protocol::{
    CodecId, EventEnvelope, FencingToken, StopReason, StreamPayload, TaskRequest, TaskResult,
    TaskState, TaskStreamEvent, WorkerStatus,
};
use futures_util::StreamExt;
use serde::Deserialize;
use tokio::sync::{Mutex, Semaphore, mpsc, watch};
use tracing::{error, info, warn};

use crate::{
    artifacts::ArtifactPackager,
    config::{BoxerConfig, now_ms},
    error::BoxerError,
    leases::{LeaseClient, LeaseSupervisor},
    preview::TraefikFileProvider,
    proxy::OutboundCredentialProxy,
    sandbox::{SandboxExecRequest, build_sandbox_driver},
};

pub use bus::{BoxerBus, InMemoryBoxerBus, NatsBoxerBus};

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CancelSignalPayload {
    task_id: String,
    #[allow(dead_code)]
    session_id: String,
    #[allow(dead_code)]
    fencing_token: u64,
}

/// Supervises sandbox execution, lease heartbeats, telemetry, and artifact manifests.
pub struct BoxerDaemon {
    config: Arc<BoxerConfig>,
    lease_client: Arc<dyn LeaseClient>,
    bus: Arc<dyn BoxerBus>,
    previews: Option<Arc<TraefikFileProvider>>,
    semaphore: Arc<Semaphore>,
    seq: Arc<AtomicU64>,
}

impl BoxerDaemon {
    /// Constructs a validated [`BoxerDaemon`].
    ///
    /// # Errors
    ///
    /// Fails closed if configuration or Traefik dynamic directory setup fails.
    pub fn new(
        config: BoxerConfig,
        lease_client: Arc<dyn LeaseClient>,
        bus: Arc<dyn BoxerBus>,
    ) -> Result<Self, BoxerError> {
        config.validate()?;
        let previews = if config.traefik_preview_enabled {
            Some(Arc::new(TraefikFileProvider::new(
                config.traefik_preview_domain.clone(),
                config.traefik_dynamic_dir.clone(),
            )?))
        } else {
            None
        };
        let max_conc = config.max_concurrent_boxes;
        Ok(Self {
            config: Arc::new(config),
            lease_client,
            bus,
            previews,
            semaphore: Arc::new(Semaphore::new(max_conc)),
            seq: Arc::new(AtomicU64::new(1)),
        })
    }

    /// Runs the NATS queue-group consumer (`boxer-workers`) until Ctrl+C shutdown.
    ///
    /// # Errors
    ///
    /// Returns [`BoxerError::Bus`] if NATS subscription fails.
    pub async fn run_nats_until_shutdown(
        self: Arc<Self>,
        nats_bus: &NatsBoxerBus,
    ) -> Result<(), BoxerError> {
        let mut sub = nats_bus
            .nats_client()
            .queue_subscribe("boxer.tasks.request", "boxer-workers".to_owned())
            .await
            .map_err(|err| BoxerError::Bus(format!("queue_subscribe failed: {err}")))?;

        let mut cancel_sub = nats_bus
            .nats_client()
            .subscribe("boxer.tasks.cancel")
            .await
            .map_err(|err| BoxerError::Bus(format!("cancel subscribe failed: {err}")))?;

        let active_cancellers: Arc<Mutex<std::collections::HashMap<String, watch::Sender<bool>>>> =
            Arc::new(Mutex::new(std::collections::HashMap::new()));

        self.emit_worker_status(
            "control-plane",
            None,
            WorkerStatus::Idle,
            Some("daemon ready"),
        )
        .await;
        info!(node_id = %self.config.node_id, "agents-boxer listening on NATS queue group boxer-workers");

        loop {
            tokio::select! {
                _ = tokio::signal::ctrl_c() => {
                    info!("received shutdown signal; stopping agents-boxer");
                    break;
                }
                Some(cancel_msg) = cancel_sub.next() => {
                    if let Ok(sig) = serde_json::from_slice::<CancelSignalPayload>(&cancel_msg.payload) {
                        let map = active_cancellers.lock().await;
                        if let Some(tx) = map.get(&sig.task_id) {
                            let _ = tx.send(true);
                        }
                    }
                }
                Some(msg) = sub.next() => {
                    match serde_json::from_slice::<TaskRequest>(&msg.payload) {
                        Ok(req) => {
                            let daemon = Arc::clone(&self);
                            let cancellers = Arc::clone(&active_cancellers);
                            tokio::spawn(async move {
                                let (ext_tx, ext_rx) = watch::channel(false);
                                cancellers.lock().await.insert(req.task_id.clone(), ext_tx);
                                let tid = req.task_id.clone();
                                if let Err(err) = daemon.execute_task(req, Some(ext_rx)).await {
                                    error!(task_id = %tid, error = %err, "task execution failed");
                                }
                                cancellers.lock().await.remove(&tid);
                            });
                        }
                        Err(err) => {
                            warn!(error = %err, "ignoring malformed TaskRequest payload");
                        }
                    }
                }
            }
        }
        Ok(())
    }

    /// Acquires a fenced lease, provisions the two-axis sandbox, executes the task,
    /// streams ordered telemetry, packages deliverables, and settles completion.
    ///
    /// # Errors
    ///
    /// Returns [`BoxerError`] on lease conflicts, isolation violations, or API errors.
    pub async fn execute_task(
        &self,
        req: TaskRequest,
        external_cancel: Option<watch::Receiver<bool>>,
    ) -> Result<TaskResult, BoxerError> {
        let _permit = self
            .semaphore
            .acquire()
            .await
            .map_err(|_| BoxerError::ExecutionFailed("worker semaphore closed".into()))?;

        let (supervisor, mut lease_cancel_rx) = LeaseSupervisor::acquire_and_start(
            Arc::clone(&self.lease_client),
            &req.task_id,
            &self.config.node_id,
            self.config.lease_ttl_seconds,
            Duration::from_secs(self.config.lease_heartbeat_seconds),
        )
        .await?;
        let token = supervisor.fencing_token();

        self.emit_worker_status(
            &req.session_id,
            Some(&req.task_id),
            WorkerStatus::Working,
            Some(&format!("leased token {}", token.0)),
        )
        .await;

        let profile = req
            .sandbox
            .clone()
            .unwrap_or_else(|| self.config.default_profile.clone());

        let driver = match build_sandbox_driver(&self.config, &profile) {
            Ok(d) => d,
            Err(err) => {
                let _ = self
                    .finish_blocked_or_failed(&req, token, &err.to_string())
                    .await;
                supervisor.stop();
                return Err(err);
            }
        };

        let _proxy = if req.exec.network_egress {
            Some(
                OutboundCredentialProxy::start(
                    self.config.proxy_bind_addr,
                    true,
                    self.config.proxy_allowed_hosts.clone(),
                    Vec::new(),
                )
                .await?,
            )
        } else {
            None
        };
        let proxy_url = _proxy.as_ref().map(OutboundCredentialProxy::endpoint_url);

        let (combined_cancel_tx, combined_cancel_rx) = watch::channel(false);
        let cancel_forwarder = tokio::spawn({
            let tx = combined_cancel_tx.clone();
            let mut ext = external_cancel;
            async move {
                tokio::select! {
                    Ok(_) = lease_cancel_rx.wait_for(|v| *v) => {
                        let _ = tx.send(true);
                    }
                    res = async {
                        if let Some(rx) = &mut ext {
                            rx.wait_for(|v| *v).await.map(|_| ())
                        } else {
                            std::future::pending::<Result<(), watch::error::RecvError>>().await
                        }
                    } => {
                        if res.is_ok() {
                            let _ = tx.send(true);
                        }
                    }
                }
            }
        });

        let (stream_tx, mut stream_rx) = mpsc::channel::<TaskStreamEvent>(128);
        let bus = Arc::clone(&self.bus);
        let seq = Arc::clone(&self.seq);
        let session_for_stream = req.session_id.clone();
        let stream_forwarder = tokio::spawn(async move {
            while let Some(ev) = stream_rx.recv().await {
                let env = EventEnvelope {
                    seq: seq.fetch_add(1, Ordering::SeqCst),
                    emitted_at_ms: ev.emitted_at_ms,
                    codec: CodecId::EventsV1,
                    session_id: session_for_stream.clone(),
                    task_id: Some(ev.task_id),
                    payload: StreamPayload::TaskChunk {
                        chunk: ev.chunk,
                        is_stderr: ev.is_stderr,
                    },
                };
                let _ = bus.publish_stream_event(&env).await;
            }
        });

        let mut exec_opts = req.exec.clone();
        if exec_opts.timeout_seconds.is_none() {
            exec_opts.timeout_seconds = req.timeout_seconds;
        }

        let exec_out = driver
            .execute(
                SandboxExecRequest {
                    task_id: req.task_id.clone(),
                    session_id: req.session_id.clone(),
                    command: req.prompt.clone(),
                    profile,
                    exec: exec_opts,
                    proxy_url,
                },
                Some(stream_tx),
                combined_cancel_rx,
            )
            .await;

        cancel_forwarder.abort();
        let _ = stream_forwarder.await;

        let fencing_lost = supervisor.is_fencing_lost();
        supervisor.stop();

        if fencing_lost {
            self.emit_worker_status(
                &req.session_id,
                Some(&req.task_id),
                WorkerStatus::Blocked,
                Some("fencing token invalidated"),
            )
            .await;
            return Err(BoxerError::FencingConflict(
                "fencing lost during execution; aborting stale completion".into(),
            ));
        }

        let output = exec_out?;
        let manifest_summary = ArtifactPackager::default()
            .package_directory(&req.task_id, &output.working_dir)
            .map(|m| format!("artifacts={} bytes={}", m.files.len(), m.total_bytes))
            .unwrap_or_else(|_| "artifacts=0".to_owned());

        let stop_reason = if output.cancelled {
            StopReason::Aborted
        } else if output.timed_out {
            StopReason::Length
        } else if output.exit_code == 0 {
            StopReason::EndTurn
        } else {
            StopReason::Error
        };

        let summary = if output.stdout.is_empty() {
            format!("exit={} ({manifest_summary})", output.exit_code)
        } else {
            format!("{} [{manifest_summary}]", output.stdout)
        };

        self.lease_client
            .complete_task(
                &req.task_id,
                token,
                output.exit_code,
                stop_reason.clone(),
                Some(summary.clone()),
            )
            .await?;

        self.emit_worker_status(
            &req.session_id,
            Some(&req.task_id),
            WorkerStatus::Done,
            Some(&format!("exit_code={}", output.exit_code)),
        )
        .await;
        self.emit_worker_status(&req.session_id, None, WorkerStatus::Idle, Some("ready"))
            .await;

        Ok(TaskResult {
            task_id: req.task_id,
            fencing_token: Some(token),
            exit_code: output.exit_code,
            stop_reason,
            output_summary: Some(summary),
            completed_at: now_ms(),
        })
    }

    #[must_use]
    pub fn preview_provider(&self) -> Option<&Arc<TraefikFileProvider>> {
        self.previews.as_ref()
    }

    async fn finish_blocked_or_failed(
        &self,
        req: &TaskRequest,
        token: FencingToken,
        reason: &str,
    ) -> Result<(), BoxerError> {
        let _ = self
            .lease_client
            .report_state(
                &req.task_id,
                token,
                TaskState::Blocked,
                Some(reason.to_owned()),
            )
            .await;
        self.emit_worker_status(
            &req.session_id,
            Some(&req.task_id),
            WorkerStatus::Blocked,
            Some(reason),
        )
        .await;
        let _ = self
            .lease_client
            .complete_task(
                &req.task_id,
                token,
                1,
                StopReason::Error,
                Some(reason.to_owned()),
            )
            .await;
        Ok(())
    }

    async fn emit_worker_status(
        &self,
        session_id: &str,
        task_id: Option<&str>,
        status: WorkerStatus,
        reason: Option<&str>,
    ) {
        let env = EventEnvelope {
            seq: self.seq.fetch_add(1, Ordering::SeqCst),
            emitted_at_ms: now_ms(),
            codec: CodecId::EventsV1,
            session_id: session_id.to_owned(),
            task_id: task_id.map(ToOwned::to_owned),
            payload: StreamPayload::WorkerStateChanged {
                node_id: self.config.node_id.clone(),
                status,
                reason: reason.map(ToOwned::to_owned),
            },
        };
        let _ = self.bus.publish_stream_event(&env).await;
    }
}
