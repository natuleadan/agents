//! Upstash QStash REST adapter implementing [`MessageBus`] and webhook signature verification.

use std::{collections::HashMap, sync::Arc};

use agents_protocol::{EventEnvelope, FencingToken, SteeringMessage, SwarmMessage, TaskRequest};
use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;
use tokio::sync::{Mutex, broadcast};

use crate::infra::{
    bus::{MessageBus, STREAM_BOXER_STREAM, STREAM_BOXER_TASKS, STREAM_SWARM_EVENTS},
    error::ServerError,
    state::BoxFuture,
};

fn compute_hmac_sha256(key: &[u8], msg: &[u8]) -> [u8; 32] {
    const BLOCK_SIZE: usize = 64;
    let mut k_block = [0u8; BLOCK_SIZE];
    if key.len() > BLOCK_SIZE {
        let hashed = Sha256::digest(key);
        k_block[..32].copy_from_slice(&hashed);
    } else {
        k_block[..key.len()].copy_from_slice(key);
    }
    let mut o_pad = [0x5cu8; BLOCK_SIZE];
    let mut i_pad = [0x36u8; BLOCK_SIZE];
    for i in 0..BLOCK_SIZE {
        o_pad[i] ^= k_block[i];
        i_pad[i] ^= k_block[i];
    }
    let mut inner = Sha256::new();
    inner.update(i_pad);
    inner.update(msg);
    let inner_hash = inner.finalize();

    let mut outer = Sha256::new();
    outer.update(o_pad);
    outer.update(inner_hash);
    outer.finalize().into()
}

/// QStash message bus implementing [`MessageBus`] via HTTP REST and in-process broadcast.
#[derive(Clone)]
pub struct QstashMessageBus {
    url: String,
    token: String,
    client: reqwest::Client,
    channels: Arc<Mutex<HashMap<String, broadcast::Sender<EventEnvelope>>>>,
}

impl QstashMessageBus {
    #[must_use]
    pub fn new(url: String, token: String) -> Self {
        Self {
            url: url.trim_end_matches('/').to_owned(),
            token,
            client: reqwest::Client::new(),
            channels: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// Verifies the `upstash-signature` header on an incoming webhook payload.
    #[must_use]
    pub fn verify_signature(
        body: &[u8],
        signature: &str,
        current_key: Option<&str>,
        next_key: Option<&str>,
    ) -> bool {
        let check_key = |key: &str| -> bool {
            let expected_bytes = compute_hmac_sha256(key.as_bytes(), body);
            matches!(
                hex::decode(signature.trim()),
                Ok(sig_bytes) if sig_bytes.len() == 32 && bool::from(sig_bytes.ct_eq(&expected_bytes))
            )
        };

        if current_key.is_some_and(check_key) {
            return true;
        }
        if next_key.is_some_and(check_key) {
            return true;
        }
        false
    }

    async fn post_publish(
        &self,
        topic: &str,
        payload: serde_json::Value,
    ) -> Result<(), ServerError> {
        let endpoint = format!("{}/v2/publish/{}", self.url, topic);
        let res = self
            .client
            .post(&endpoint)
            .bearer_auth(&self.token)
            .json(&payload)
            .send()
            .await
            .map_err(|e| ServerError::Internal(format!("qstash publish network error: {e}")))?;

        if !res.status().is_success() {
            let status = res.status();
            let txt = res.text().await.unwrap_or_default();
            return Err(ServerError::Internal(format!(
                "qstash publish failed {status}: {txt}"
            )));
        }
        Ok(())
    }
}

impl MessageBus for QstashMessageBus {
    fn publish_task_request<'a>(
        &'a self,
        req: &'a TaskRequest,
    ) -> BoxFuture<'a, Result<(), ServerError>> {
        Box::pin(async move {
            let payload = serde_json::to_value(req)
                .map_err(|e| ServerError::Internal(format!("serialize task request: {e}")))?;
            self.post_publish(STREAM_BOXER_TASKS, payload).await
        })
    }

    fn publish_task_cancel<'a>(
        &'a self,
        task_id: &'a str,
        session_id: &'a str,
        fencing_token: FencingToken,
    ) -> BoxFuture<'a, Result<(), ServerError>> {
        Box::pin(async move {
            let payload = serde_json::json!({
                "type": "cancel",
                "taskId": task_id,
                "sessionId": session_id,
                "fencingToken": fencing_token.0,
            });
            self.post_publish(STREAM_BOXER_TASKS, payload).await
        })
    }

    fn publish_steering<'a>(
        &'a self,
        msg: &'a SteeringMessage,
    ) -> BoxFuture<'a, Result<(), ServerError>> {
        Box::pin(async move {
            let payload = serde_json::to_value(msg)
                .map_err(|e| ServerError::Internal(format!("serialize steering msg: {e}")))?;
            self.post_publish(STREAM_BOXER_TASKS, payload).await
        })
    }

    fn publish_stream_event<'a>(
        &'a self,
        envelope: &'a EventEnvelope,
    ) -> BoxFuture<'a, Result<(), ServerError>> {
        Box::pin(async move {
            let session_id = envelope.session_id.clone();
            let channels = self.channels.lock().await;
            if let Some(tx) = channels.get(&session_id) {
                let _ = tx.send(envelope.clone());
            }
            drop(channels);

            let payload = serde_json::to_value(envelope)
                .map_err(|e| ServerError::Internal(format!("serialize stream event: {e}")))?;
            let _ = self.post_publish(STREAM_BOXER_STREAM, payload).await;
            Ok(())
        })
    }

    fn publish_swarm_message<'a>(
        &'a self,
        msg: &'a SwarmMessage,
    ) -> BoxFuture<'a, Result<(), ServerError>> {
        Box::pin(async move {
            let payload = serde_json::to_value(msg)
                .map_err(|e| ServerError::Internal(format!("serialize swarm msg: {e}")))?;
            self.post_publish(STREAM_SWARM_EVENTS, payload).await
        })
    }

    fn subscribe_session_stream<'a>(
        &'a self,
        session_id: &'a str,
    ) -> BoxFuture<'a, broadcast::Receiver<EventEnvelope>> {
        Box::pin(async move {
            let mut channels = self.channels.lock().await;
            let tx = channels
                .entry(session_id.to_owned())
                .or_insert_with(|| broadcast::channel(512).0);
            tx.subscribe()
        })
    }

    fn ping<'a>(&'a self) -> BoxFuture<'a, Result<(), ServerError>> {
        Box::pin(async move {
            let endpoint = format!("{}/v2/topics", self.url);
            let res = self
                .client
                .get(&endpoint)
                .bearer_auth(&self.token)
                .send()
                .await
                .map_err(|e| ServerError::Internal(format!("qstash ping network: {e}")))?;
            if res.status().is_success() {
                Ok(())
            } else {
                Err(ServerError::Internal(format!(
                    "qstash ping failed with status {}",
                    res.status()
                )))
            }
        })
    }
}
