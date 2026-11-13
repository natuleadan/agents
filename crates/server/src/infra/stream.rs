//! Durable per-session ordered `EventEnvelope` persistence and live bus fanout.

use std::{collections::HashMap, sync::Arc};

use agents_protocol::{CodecId, EventEnvelope, StreamPayload};
use sqlx::{PgPool, Row};
use tokio::sync::Mutex;

use crate::infra::{
    bus::MessageBus,
    error::ServerError,
    state::{BoxFuture, now_ms},
};

/// Concrete DI wrapper around [`StreamRepository`].
#[derive(Clone)]
pub struct StreamStore(pub Arc<dyn StreamRepository>);

impl Default for StreamStore {
    fn default() -> Self {
        Self(InMemoryStreamRepository::new())
    }
}

/// Concrete DI wrapper around [`MessageBus`].
#[derive(Clone)]
pub struct BusClient(pub Arc<dyn MessageBus>);

impl Default for BusClient {
    fn default() -> Self {
        Self(crate::infra::bus::InMemoryMessageBus::new())
    }
}

/// Concrete DI wrapper around [`crate::infra::admission::AdmissionController`].
#[derive(Clone)]
pub struct AdmissionStore(pub Arc<crate::infra::admission::AdmissionController>);

impl Default for AdmissionStore {
    fn default() -> Self {
        Self(crate::infra::admission::AdmissionController::new(64, 4))
    }
}

/// Persistence contract for ordered per-session stream envelopes.
pub trait StreamRepository: Send + Sync + 'static {
    fn append_next<'a>(
        &'a self,
        session_id: &'a str,
        task_id: Option<&'a str>,
        payload: StreamPayload,
    ) -> BoxFuture<'a, Result<EventEnvelope, ServerError>>;

    fn list_after_seq<'a>(
        &'a self,
        session_id: &'a str,
        after_seq: u64,
        limit: u32,
    ) -> BoxFuture<'a, Result<Vec<EventEnvelope>, ServerError>>;
}

/// PostgreSQL stream event repository assigning monotonic per-session `seq`.
#[derive(Clone)]
pub struct PgStreamRepository {
    pool: PgPool,
}

impl PgStreamRepository {
    #[must_use]
    pub fn new(pool: PgPool) -> Arc<Self> {
        Arc::new(Self { pool })
    }
}

impl StreamRepository for PgStreamRepository {
    fn append_next<'a>(
        &'a self,
        session_id: &'a str,
        task_id: Option<&'a str>,
        payload: StreamPayload,
    ) -> BoxFuture<'a, Result<EventEnvelope, ServerError>> {
        Box::pin(async move {
            let mut tx = self
                .pool
                .begin()
                .await
                .map_err(|e| ServerError::Internal(format!("stream tx begin failed: {e}")))?;

            let row = sqlx::query(
                "SELECT COALESCE(MAX(seq), 0) AS max_seq FROM stream_events WHERE session_id = $1",
            )
            .bind(session_id)
            .fetch_one(&mut *tx)
            .await
            .map_err(|e| ServerError::Internal(format!("stream max_seq query failed: {e}")))?;

            let current_seq: i64 = row
                .try_get("max_seq")
                .map_err(|e| ServerError::Internal(e.to_string()))?;
            let next_seq = (current_seq.max(0) as u64).saturating_add(1);
            let emitted_at_ms = now_ms();
            let codec = CodecId::EventsV1;
            let payload_json =
                serde_json::to_value(&payload).map_err(|e| ServerError::Internal(e.to_string()))?;

            sqlx::query(
                "INSERT INTO stream_events (session_id, task_id, seq, emitted_at_ms, codec, payload)
                 VALUES ($1, $2, $3, $4, $5, $6)",
            )
            .bind(session_id)
            .bind(task_id)
            .bind(next_seq as i64)
            .bind(emitted_at_ms as i64)
            .bind(codec.as_str())
            .bind(payload_json)
            .execute(&mut *tx)
            .await
            .map_err(|e| ServerError::Internal(format!("stream insert failed: {e}")))?;

            tx.commit()
                .await
                .map_err(|e| ServerError::Internal(format!("stream commit failed: {e}")))?;

            Ok(EventEnvelope {
                seq: next_seq,
                emitted_at_ms,
                codec,
                session_id: session_id.to_owned(),
                task_id: task_id.map(ToOwned::to_owned),
                payload,
            })
        })
    }

    fn list_after_seq<'a>(
        &'a self,
        session_id: &'a str,
        after_seq: u64,
        limit: u32,
    ) -> BoxFuture<'a, Result<Vec<EventEnvelope>, ServerError>> {
        Box::pin(async move {
            let rows = sqlx::query(
                "SELECT session_id, task_id, seq, emitted_at_ms, codec, payload
                 FROM stream_events
                 WHERE session_id = $1 AND seq > $2
                 ORDER BY seq ASC
                 LIMIT $3",
            )
            .bind(session_id)
            .bind(after_seq as i64)
            .bind(i64::from(limit.clamp(1, 1000)))
            .fetch_all(&self.pool)
            .await
            .map_err(|e| ServerError::Internal(format!("stream list failed: {e}")))?;

            rows.into_iter().map(row_to_envelope).collect()
        })
    }
}

fn row_to_envelope(row: sqlx::postgres::PgRow) -> Result<EventEnvelope, ServerError> {
    let seq: i64 = row
        .try_get("seq")
        .map_err(|e| ServerError::Internal(e.to_string()))?;
    let emitted: i64 = row
        .try_get("emitted_at_ms")
        .map_err(|e| ServerError::Internal(e.to_string()))?;
    let codec_str: String = row
        .try_get("codec")
        .map_err(|e| ServerError::Internal(e.to_string()))?;
    let payload_val: serde_json::Value = row
        .try_get("payload")
        .map_err(|e| ServerError::Internal(e.to_string()))?;
    let payload: StreamPayload =
        serde_json::from_value(payload_val).map_err(|e| ServerError::Internal(e.to_string()))?;

    Ok(EventEnvelope {
        seq: seq.max(0) as u64,
        emitted_at_ms: emitted.max(0) as u64,
        codec: match codec_str.as_str() {
            "snapshot.v1" => CodecId::SnapshotV1,
            "events.v1" => CodecId::EventsV1,
            other => CodecId::Unknown(other.to_owned()),
        },
        session_id: row
            .try_get("session_id")
            .map_err(|e| ServerError::Internal(e.to_string()))?,
        task_id: row
            .try_get("task_id")
            .map_err(|e| ServerError::Internal(e.to_string()))?,
        payload,
    })
}

/// Deterministic in-memory stream event repository for tests.
#[derive(Default)]
pub struct InMemoryStreamRepository {
    events: Mutex<HashMap<String, Vec<EventEnvelope>>>,
}

impl InMemoryStreamRepository {
    #[must_use]
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }
}

impl StreamRepository for InMemoryStreamRepository {
    fn append_next<'a>(
        &'a self,
        session_id: &'a str,
        task_id: Option<&'a str>,
        payload: StreamPayload,
    ) -> BoxFuture<'a, Result<EventEnvelope, ServerError>> {
        Box::pin(async move {
            let mut guard = self.events.lock().await;
            let list = guard.entry(session_id.to_owned()).or_default();
            let seq = list.last().map_or(1, |e| e.seq.saturating_add(1));
            let env = EventEnvelope {
                seq,
                emitted_at_ms: now_ms(),
                codec: CodecId::EventsV1,
                session_id: session_id.to_owned(),
                task_id: task_id.map(ToOwned::to_owned),
                payload,
            };
            list.push(env.clone());
            Ok(env)
        })
    }

    fn list_after_seq<'a>(
        &'a self,
        session_id: &'a str,
        after_seq: u64,
        limit: u32,
    ) -> BoxFuture<'a, Result<Vec<EventEnvelope>, ServerError>> {
        Box::pin(async move {
            let guard = self.events.lock().await;
            let items = guard
                .get(session_id)
                .map(|list| {
                    list.iter()
                        .filter(|e| e.seq > after_seq)
                        .take(limit.clamp(1, 1000) as usize)
                        .cloned()
                        .collect()
                })
                .unwrap_or_default();
            Ok(items)
        })
    }
}

/// Helper combining durable stream persistence and NATS/local fanout.
pub async fn emit_stream_event(
    stream_store: &StreamStore,
    bus: &BusClient,
    session_id: &str,
    task_id: Option<&str>,
    payload: StreamPayload,
) -> Result<EventEnvelope, ServerError> {
    let envelope = stream_store
        .0
        .append_next(session_id, task_id, payload)
        .await?;
    bus.0.publish_stream_event(&envelope).await?;
    Ok(envelope)
}
