//! NATS JetStream and deterministic in-memory worker message bus implementations.

use std::sync::Arc;

use agents_protocol::EventEnvelope;
use tokio::sync::Mutex;

use crate::{config::BoxerConfig, error::BoxerError, leases::BoxFuture};

/// Abstract worker message bus linking Boxer to NATS JetStream or an in-memory harness.
pub trait BoxerBus: Send + Sync + 'static {
    fn publish_stream_event<'a>(
        &'a self,
        envelope: &'a EventEnvelope,
    ) -> BoxFuture<'a, Result<(), BoxerError>>;
}

/// NATS JetStream bus client running over private VLAN with queue-group delivery.
#[derive(Clone)]
pub struct NatsBoxerBus {
    client: async_nats::Client,
    jetstream: async_nats::jetstream::Context,
}

impl NatsBoxerBus {
    /// Connects to NATS JetStream.
    ///
    /// # Errors
    ///
    /// Returns [`BoxerError::Bus`] if the connection cannot be established.
    pub async fn connect(cfg: &BoxerConfig) -> Result<Arc<Self>, BoxerError> {
        let client = async_nats::connect(&cfg.nats_url)
            .await
            .map_err(|err| BoxerError::Bus(format!("nats connect failed: {err}")))?;
        let jetstream = async_nats::jetstream::new(client.clone());
        Ok(Arc::new(Self { client, jetstream }))
    }

    #[must_use]
    pub fn nats_client(&self) -> &async_nats::Client {
        &self.client
    }
}

impl BoxerBus for NatsBoxerBus {
    fn publish_stream_event<'a>(
        &'a self,
        envelope: &'a EventEnvelope,
    ) -> BoxFuture<'a, Result<(), BoxerError>> {
        Box::pin(async move {
            let payload = serde_json::to_vec(envelope)
                .map_err(|err| BoxerError::Bus(format!("serialize envelope failed: {err}")))?;
            let subject = format!("boxer.stream.{}", envelope.session_id);
            self.jetstream
                .publish(subject, payload.into())
                .await
                .map_err(|err| BoxerError::Bus(format!("nats publish failed: {err}")))?
                .await
                .map_err(|err| BoxerError::Bus(format!("nats ack failed: {err}")))?;
            Ok(())
        })
    }
}

/// Deterministic in-memory bus recording telemetry emitted during hermetic tests.
#[derive(Clone, Default)]
pub struct InMemoryBoxerBus {
    events: Arc<Mutex<Vec<EventEnvelope>>>,
}

impl InMemoryBoxerBus {
    #[must_use]
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    pub async fn recorded_events(&self) -> Vec<EventEnvelope> {
        self.events.lock().await.clone()
    }
}

impl BoxerBus for InMemoryBoxerBus {
    fn publish_stream_event<'a>(
        &'a self,
        envelope: &'a EventEnvelope,
    ) -> BoxFuture<'a, Result<(), BoxerError>> {
        Box::pin(async move {
            self.events.lock().await.push(envelope.clone());
            Ok(())
        })
    }
}
