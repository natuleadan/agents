//! Control-plane lease API trait, HTTP VLAN client, and deterministic in-memory store.

use std::{
    collections::HashMap,
    future::Future,
    pin::Pin,
    sync::{Arc, atomic::Ordering},
};

use agents_protocol::{FencingToken, StopReason, TaskLease, TaskState};
use bytes::Bytes;
use http::{Request, header};
use http_body_util::{BodyExt, Full};
use hyper_util::{
    client::legacy::{Client, connect::HttpConnector},
    rt::TokioExecutor,
};
use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;

use crate::{config::now_ms, error::BoxerError};

pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// Control-plane lease contract invoked by Boxer daemons when claiming and settling tasks.
pub trait LeaseClient: Send + Sync + 'static {
    fn acquire_lease<'a>(
        &'a self,
        task_id: &'a str,
        boxer_node_id: &'a str,
        lease_ttl_seconds: u64,
    ) -> BoxFuture<'a, Result<TaskLease, BoxerError>>;

    fn report_state<'a>(
        &'a self,
        task_id: &'a str,
        fencing_token: FencingToken,
        state: TaskState,
        reason: Option<String>,
    ) -> BoxFuture<'a, Result<(), BoxerError>>;

    fn complete_task<'a>(
        &'a self,
        task_id: &'a str,
        fencing_token: FencingToken,
        exit_code: i32,
        stop_reason: StopReason,
        output_summary: Option<String>,
    ) -> BoxFuture<'a, Result<(), BoxerError>>;
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct LeaseRequestPayload<'a> {
    boxer_node_id: &'a str,
    lease_ttl_seconds: u64,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct UpdateStatePayload {
    fencing_token: FencingToken,
    state: TaskState,
    reason: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct CompleteTaskPayload {
    fencing_token: FencingToken,
    exit_code: i32,
    stop_reason: StopReason,
    output_summary: Option<String>,
}

/// HTTP client talking over private VLAN (`SERVER_INTERNAL_URL`) to Server `/api/v1/tasks/:id/*`.
#[derive(Clone)]
pub struct HttpServerLeaseClient {
    base_url: String,
    worker_token: String,
    http: Client<HttpConnector, Full<Bytes>>,
}

impl HttpServerLeaseClient {
    #[must_use]
    pub fn new(base_url: impl Into<String>, worker_token: impl Into<String>) -> Self {
        let mut connector = HttpConnector::new();
        connector.enforce_http(false);
        let http = Client::builder(TokioExecutor::new()).build(connector);
        Self {
            base_url: base_url.into().trim_end_matches('/').to_owned(),
            worker_token: worker_token.into(),
            http,
        }
    }

    async fn post_json<Req: Serialize, Resp: for<'de> Deserialize<'de>>(
        &self,
        path: &str,
        body: &Req,
    ) -> Result<Resp, BoxerError> {
        let raw = self
            .post_raw(path, serde_json::to_vec(body).map_err(json_err)?)
            .await?;
        serde_json::from_slice(&raw).map_err(json_err)
    }

    async fn post_raw(&self, path: &str, body: Vec<u8>) -> Result<Vec<u8>, BoxerError> {
        let uri = format!("{}{}", self.base_url, path);
        let req = Request::post(&uri)
            .header(header::CONTENT_TYPE, "application/json")
            .header(
                header::AUTHORIZATION,
                format!("Bearer {}", self.worker_token),
            )
            .body(Full::new(Bytes::from(body)))
            .map_err(|err| BoxerError::ServerApi(format!("invalid request `{uri}`: {err}")))?;

        let resp =
            self.http.request(req).await.map_err(|err| {
                BoxerError::ServerApi(format!("request to `{uri}` failed: {err}"))
            })?;

        let status = resp.status();
        let bytes = resp
            .into_body()
            .collect()
            .await
            .map_err(|err| BoxerError::ServerApi(format!("failed reading response body: {err}")))?
            .to_bytes()
            .to_vec();

        if status.as_u16() == 409 {
            return Err(BoxerError::FencingConflict(
                String::from_utf8_lossy(&bytes).into_owned(),
            ));
        }
        if !status.is_success() {
            return Err(BoxerError::ServerApi(format!(
                "server returned HTTP {} on `{path}`: {}",
                status.as_u16(),
                String::from_utf8_lossy(&bytes)
            )));
        }
        Ok(bytes)
    }
}

fn json_err(err: serde_json::Error) -> BoxerError {
    BoxerError::ServerApi(format!("json serialization error: {err}"))
}

impl LeaseClient for HttpServerLeaseClient {
    fn acquire_lease<'a>(
        &'a self,
        task_id: &'a str,
        boxer_node_id: &'a str,
        lease_ttl_seconds: u64,
    ) -> BoxFuture<'a, Result<TaskLease, BoxerError>> {
        Box::pin(async move {
            self.post_json(
                &format!("/api/v1/tasks/{task_id}/lease"),
                &LeaseRequestPayload {
                    boxer_node_id,
                    lease_ttl_seconds,
                },
            )
            .await
        })
    }

    fn report_state<'a>(
        &'a self,
        task_id: &'a str,
        fencing_token: FencingToken,
        state: TaskState,
        reason: Option<String>,
    ) -> BoxFuture<'a, Result<(), BoxerError>> {
        Box::pin(async move {
            let payload = UpdateStatePayload {
                fencing_token,
                state,
                reason,
            };
            let _ = self
                .post_raw(
                    &format!("/api/v1/tasks/{task_id}/state"),
                    serde_json::to_vec(&payload).map_err(json_err)?,
                )
                .await?;
            Ok(())
        })
    }

    fn complete_task<'a>(
        &'a self,
        task_id: &'a str,
        fencing_token: FencingToken,
        exit_code: i32,
        stop_reason: StopReason,
        output_summary: Option<String>,
    ) -> BoxFuture<'a, Result<(), BoxerError>> {
        Box::pin(async move {
            let payload = CompleteTaskPayload {
                fencing_token,
                exit_code,
                stop_reason,
                output_summary,
            };
            let _ = self
                .post_raw(
                    &format!("/api/v1/tasks/{task_id}/complete"),
                    serde_json::to_vec(&payload).map_err(json_err)?,
                )
                .await?;
            Ok(())
        })
    }
}

#[derive(Debug, Clone)]
struct StoredLease {
    session_id: String,
    node_id: String,
    token: FencingToken,
    expires_at_ms: u64,
    state: TaskState,
    exit_code: Option<i32>,
    stop_reason: Option<StopReason>,
    output_summary: Option<String>,
}

/// Deterministic in-memory [`LeaseClient`] for hermetic worker & fencing tests.
#[derive(Clone, Default)]
pub struct InMemoryLeaseClient {
    inner: Arc<Mutex<HashMap<String, StoredLease>>>,
    next_token: Arc<std::sync::atomic::AtomicU64>,
}

impl InMemoryLeaseClient {
    #[must_use]
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            inner: Arc::new(Mutex::new(HashMap::new())),
            next_token: Arc::new(std::sync::atomic::AtomicU64::new(1)),
        })
    }

    /// Registers a queued task for test acquisition.
    pub async fn seed_task(&self, task_id: &str, session_id: &str) {
        let mut guard = self.inner.lock().await;
        guard.insert(
            task_id.to_owned(),
            StoredLease {
                session_id: session_id.to_owned(),
                node_id: String::new(),
                token: FencingToken(0),
                expires_at_ms: 0,
                state: TaskState::Queued,
                exit_code: None,
                stop_reason: None,
                output_summary: None,
            },
        );
    }

    /// Simulates lease superseding by another worker with a newer monotonic fencing token.
    ///
    /// # Errors
    ///
    /// Returns [`BoxerError::ServerApi`] when `task_id` is unknown.
    pub async fn force_bump_token(
        &self,
        task_id: &str,
        new_holder: &str,
    ) -> Result<FencingToken, BoxerError> {
        let mut guard = self.inner.lock().await;
        let entry = guard
            .get_mut(task_id)
            .ok_or_else(|| BoxerError::ServerApi(format!("unknown task `{task_id}`")))?;
        let token = FencingToken(self.next_token.fetch_add(1, Ordering::SeqCst));
        entry.token = token;
        entry.node_id = new_holder.to_owned();
        entry.expires_at_ms = now_ms().saturating_add(120_000);
        Ok(token)
    }

    /// Returns recorded completion tuple `(state, exit_code, stop_reason, summary)` for assertions.
    pub async fn recorded_completion(
        &self,
        task_id: &str,
    ) -> Option<(TaskState, Option<i32>, Option<StopReason>, Option<String>)> {
        self.inner.lock().await.get(task_id).map(|entry| {
            (
                entry.state.clone(),
                entry.exit_code,
                entry.stop_reason.clone(),
                entry.output_summary.clone(),
            )
        })
    }
}

impl LeaseClient for InMemoryLeaseClient {
    fn acquire_lease<'a>(
        &'a self,
        task_id: &'a str,
        boxer_node_id: &'a str,
        lease_ttl_seconds: u64,
    ) -> BoxFuture<'a, Result<TaskLease, BoxerError>> {
        Box::pin(async move {
            let now = now_ms();
            let expires = now.saturating_add(lease_ttl_seconds.saturating_mul(1000));
            let mut guard = self.inner.lock().await;
            let entry = guard
                .entry(task_id.to_owned())
                .or_insert_with(|| StoredLease {
                    session_id: "default-session".to_owned(),
                    node_id: String::new(),
                    token: FencingToken(0),
                    expires_at_ms: 0,
                    state: TaskState::Queued,
                    exit_code: None,
                    stop_reason: None,
                    output_summary: None,
                });
            if matches!(entry.state, TaskState::Leased | TaskState::Running)
                && entry.expires_at_ms > now
            {
                return Err(BoxerError::FencingConflict(format!(
                    "task `{task_id}` actively leased by `{}`",
                    entry.node_id
                )));
            }
            let token = FencingToken(self.next_token.fetch_add(1, Ordering::SeqCst));
            entry.token = token;
            entry.node_id = boxer_node_id.to_owned();
            entry.expires_at_ms = expires;
            entry.state = TaskState::Leased;
            Ok(TaskLease {
                task_id: task_id.to_owned(),
                session_id: entry.session_id.clone(),
                boxer_node_id: boxer_node_id.to_owned(),
                fencing_token: token,
                acquired_at_ms: now,
                expires_at_ms: expires,
            })
        })
    }

    fn report_state<'a>(
        &'a self,
        task_id: &'a str,
        fencing_token: FencingToken,
        state: TaskState,
        _reason: Option<String>,
    ) -> BoxFuture<'a, Result<(), BoxerError>> {
        Box::pin(async move {
            let now = now_ms();
            let mut guard = self.inner.lock().await;
            let entry = guard
                .get_mut(task_id)
                .ok_or_else(|| BoxerError::ServerApi(format!("unknown task `{task_id}`")))?;
            if entry.token != fencing_token {
                return Err(BoxerError::FencingConflict(format!(
                    "stale fencing token {} (active {})",
                    fencing_token.0, entry.token.0
                )));
            }
            if entry.expires_at_ms <= now {
                return Err(BoxerError::LeaseExpired(format!(
                    "lease expired for task `{task_id}`"
                )));
            }
            entry.state = state;
            Ok(())
        })
    }

    fn complete_task<'a>(
        &'a self,
        task_id: &'a str,
        fencing_token: FencingToken,
        exit_code: i32,
        stop_reason: StopReason,
        output_summary: Option<String>,
    ) -> BoxFuture<'a, Result<(), BoxerError>> {
        Box::pin(async move {
            let mut guard = self.inner.lock().await;
            let entry = guard
                .get_mut(task_id)
                .ok_or_else(|| BoxerError::ServerApi(format!("unknown task `{task_id}`")))?;
            if entry.token != fencing_token {
                return Err(BoxerError::FencingConflict(format!(
                    "stale fencing token {} on completion",
                    fencing_token.0
                )));
            }
            entry.state = if exit_code == 0 {
                TaskState::Done
            } else {
                TaskState::Failed
            };
            entry.exit_code = Some(exit_code);
            entry.stop_reason = Some(stop_reason);
            entry.output_summary = output_summary;
            Ok(())
        })
    }
}
