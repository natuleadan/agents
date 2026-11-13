//! Active lease supervisor running background heartbeats and tripping cancellation
//! immediately on fencing conflict or TTL expiration.

use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

use agents_protocol::{FencingToken, TaskLease, TaskState};
use tokio::{sync::watch, task::JoinHandle};
use tracing::warn;

use crate::{config::now_ms, error::BoxerError, leases::LeaseClient};

/// Active lease supervisor running background heartbeats and tripping cancellation
/// immediately on fencing conflict or TTL expiration.
pub struct LeaseSupervisor {
    lease: TaskLease,
    cancelled: Arc<AtomicBool>,
    cancel_tx: watch::Sender<bool>,
    heartbeat_task: Option<JoinHandle<()>>,
}

impl LeaseSupervisor {
    /// Acquires a lease and starts background periodic heartbeats.
    ///
    /// # Errors
    ///
    /// Returns [`BoxerError::FencingConflict`] if already held by another worker.
    pub async fn acquire_and_start(
        client: Arc<dyn LeaseClient>,
        task_id: &str,
        node_id: &str,
        ttl_seconds: u64,
        heartbeat_interval: Duration,
    ) -> Result<(Self, watch::Receiver<bool>), BoxerError> {
        let lease = client.acquire_lease(task_id, node_id, ttl_seconds).await?;
        client
            .report_state(
                task_id,
                lease.fencing_token,
                TaskState::Running,
                Some(format!("leased by {node_id}")),
            )
            .await?;

        let cancelled = Arc::new(AtomicBool::new(false));
        let (cancel_tx, cancel_rx) = watch::channel(false);

        let hb_client = client;
        let hb_task_id = lease.task_id.clone();
        let hb_token = lease.fencing_token;
        let hb_expires = lease.expires_at_ms;
        let hb_cancelled = Arc::clone(&cancelled);
        let hb_tx = cancel_tx.clone();

        let heartbeat_task = tokio::spawn(async move {
            let mut ticker = tokio::time::interval(heartbeat_interval);
            ticker.tick().await;
            loop {
                ticker.tick().await;
                if hb_cancelled.load(Ordering::SeqCst) {
                    break;
                }
                if now_ms() >= hb_expires {
                    warn!(task_id = %hb_task_id, "lease TTL elapsed locally; tripping cancellation");
                    hb_cancelled.store(true, Ordering::SeqCst);
                    let _ = hb_tx.send(true);
                    break;
                }
                if let Err(err) = hb_client
                    .report_state(
                        &hb_task_id,
                        hb_token,
                        TaskState::Running,
                        Some("heartbeat".to_owned()),
                    )
                    .await
                {
                    warn!(
                        task_id = %hb_task_id,
                        error = %err,
                        "lease heartbeat rejected; tripping cancellation"
                    );
                    hb_cancelled.store(true, Ordering::SeqCst);
                    let _ = hb_tx.send(true);
                    break;
                }
            }
        });

        Ok((
            Self {
                lease,
                cancelled,
                cancel_tx,
                heartbeat_task: Some(heartbeat_task),
            },
            cancel_rx,
        ))
    }

    #[must_use]
    pub fn lease(&self) -> &TaskLease {
        &self.lease
    }

    #[must_use]
    pub fn fencing_token(&self) -> FencingToken {
        self.lease.fencing_token
    }

    #[must_use]
    pub fn is_fencing_lost(&self) -> bool {
        self.cancelled.load(Ordering::SeqCst) || now_ms() >= self.lease.expires_at_ms
    }

    /// Stops heartbeat task without failing callers.
    pub fn stop(mut self) {
        self.cancelled.store(true, Ordering::SeqCst);
        let _ = self.cancel_tx.send(true);
        if let Some(handle) = self.heartbeat_task.take() {
            handle.abort();
        }
    }
}

impl Drop for LeaseSupervisor {
    fn drop(&mut self) {
        self.cancelled.store(true, Ordering::SeqCst);
        let _ = self.cancel_tx.send(true);
        if let Some(handle) = self.heartbeat_task.take() {
            handle.abort();
        }
    }
}
