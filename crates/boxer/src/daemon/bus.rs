//! NATS JetStream and deterministic in-memory worker message bus implementations.

use std::sync::Arc;

use agents_protocol::{BoxerNodeAnnouncement, EventEnvelope, PreviewRouteSync};
use tokio::sync::Mutex;

use crate::{config::BoxerConfig, error::BoxerError, leases::BoxFuture};

/// Abstract worker message bus linking Boxer to NATS JetStream or an in-memory harness.
pub trait BoxerBus: Send + Sync + 'static {
    fn publish_stream_event<'a>(
        &'a self,
        envelope: &'a EventEnvelope,
    ) -> BoxFuture<'a, Result<(), BoxerError>>;

    fn publish_node_announcement<'a>(
        &'a self,
        announcement: &'a BoxerNodeAnnouncement,
    ) -> BoxFuture<'a, Result<(), BoxerError>>;

    fn publish_preview_sync<'a>(
        &'a self,
        sync: &'a PreviewRouteSync,
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

    fn publish_node_announcement<'a>(
        &'a self,
        announcement: &'a BoxerNodeAnnouncement,
    ) -> BoxFuture<'a, Result<(), BoxerError>> {
        Box::pin(async move {
            let payload = serde_json::to_vec(announcement)
                .map_err(|err| BoxerError::Bus(format!("serialize announcement failed: {err}")))?;
            let subject = format!("boxer.nodes.{}", announcement.node_id);
            self.client
                .publish(subject, payload.into())
                .await
                .map_err(|err| BoxerError::Bus(format!("nats node publish failed: {err}")))?;
            Ok(())
        })
    }

    fn publish_preview_sync<'a>(
        &'a self,
        sync: &'a PreviewRouteSync,
    ) -> BoxFuture<'a, Result<(), BoxerError>> {
        Box::pin(async move {
            let payload = serde_json::to_vec(sync)
                .map_err(|err| BoxerError::Bus(format!("serialize preview sync failed: {err}")))?;
            let subject = format!("boxer.previews.{}", sync.route.route_id);
            self.client
                .publish(subject, payload.into())
                .await
                .map_err(|err| BoxerError::Bus(format!("nats preview publish failed: {err}")))?;
            Ok(())
        })
    }
}

/// Deterministic in-memory bus recording telemetry emitted during hermetic tests.
#[derive(Clone, Default)]
pub struct InMemoryBoxerBus {
    events: Arc<Mutex<Vec<EventEnvelope>>>,
    announcements: Arc<Mutex<Vec<BoxerNodeAnnouncement>>>,
    preview_syncs: Arc<Mutex<Vec<PreviewRouteSync>>>,
}

impl InMemoryBoxerBus {
    #[must_use]
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    pub async fn recorded_events(&self) -> Vec<EventEnvelope> {
        self.events.lock().await.clone()
    }

    pub async fn recorded_announcements(&self) -> Vec<BoxerNodeAnnouncement> {
        self.announcements.lock().await.clone()
    }

    pub async fn recorded_preview_syncs(&self) -> Vec<PreviewRouteSync> {
        self.preview_syncs.lock().await.clone()
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

    fn publish_node_announcement<'a>(
        &'a self,
        announcement: &'a BoxerNodeAnnouncement,
    ) -> BoxFuture<'a, Result<(), BoxerError>> {
        Box::pin(async move {
            self.announcements.lock().await.push(announcement.clone());
            Ok(())
        })
    }

    fn publish_preview_sync<'a>(
        &'a self,
        sync: &'a PreviewRouteSync,
    ) -> BoxFuture<'a, Result<(), BoxerError>> {
        Box::pin(async move {
            self.preview_syncs.lock().await.push(sync.clone());
            Ok(())
        })
    }
}
