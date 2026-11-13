//! Hexagonal infrastructure registry unifying ports for 999 third-party providers.

use std::sync::Arc;

use crate::infra::{
    adapters::{
        blob::{BlobStorePort, LocalFsBlobStore, UpstashBlobStore},
        qstash::QstashMessageBus,
        vector::{InMemoryVectorStore, UpstashVectorAdapter, VectorStorePort},
    },
    bus::{InMemoryMessageBus, MessageBus, NatsMessageBus},
    config::{InfrastructureMode, ServerConfig},
    error::ServerError,
    state::{InMemoryStateBackend, StateBackend, ValkeyStateBackend},
};

/// Injectable wrapper for [`VectorStorePort`] in Ironic's dependency container.
#[derive(Clone)]
pub struct VectorStoreClient(pub Arc<dyn VectorStorePort>);

impl Default for VectorStoreClient {
    fn default() -> Self {
        Self(Arc::new(InMemoryVectorStore::new()))
    }
}

/// Injectable wrapper for [`BlobStorePort`] in Ironic's dependency container.
#[derive(Clone)]
pub struct BlobStoreClient(pub Arc<dyn BlobStorePort>);

impl Default for BlobStoreClient {
    fn default() -> Self {
        Self(Arc::new(LocalFsBlobStore::new(std::path::PathBuf::from(
            "./target/test-blobs",
        ))))
    }
}

/// Centralized hexagonal registry providing abstract ports across 999 infrastructure providers.
#[derive(Clone)]
pub struct InfrastructureRegistry {
    pub mode: InfrastructureMode,
    pub state: Arc<dyn StateBackend>,
    pub bus: Arc<dyn MessageBus>,
    pub vector: Arc<dyn VectorStorePort>,
    pub blob: Arc<dyn BlobStorePort>,
}

impl InfrastructureRegistry {
    /// Bootstraps adapters based strictly on [`ServerConfig::infrastructure_mode`] without hot runtime mixing.
    ///
    /// # Errors
    ///
    /// Returns [`ServerError`] when adapter initialization fails.
    pub async fn from_config(config: &ServerConfig) -> Result<Self, ServerError> {
        let mode = config.infrastructure_mode;

        let state: Arc<dyn StateBackend> = if let Some(url) = config
            .upstash_redis_url
            .as_deref()
            .or(config.valkey_url.as_deref())
        {
            ValkeyStateBackend::connect(url).await?
        } else {
            InMemoryStateBackend::new()
        };

        let bus: Arc<dyn MessageBus> = match (mode, &config.qstash_url, &config.nats_url) {
            (InfrastructureMode::ManagedCloud, Some(q_url), _) => {
                let token = config.qstash_token.clone().unwrap_or_default();
                Arc::new(QstashMessageBus::new(q_url.clone(), token))
            }
            (_, _, Some(nats_url)) => NatsMessageBus::connect(nats_url).await?,
            _ => InMemoryMessageBus::new(),
        };

        let vector: Arc<dyn VectorStorePort> = if let (Some(url), Some(token)) = (
            &config.upstash_vector_rest_url,
            &config.upstash_vector_rest_token,
        ) {
            Arc::new(UpstashVectorAdapter::new(url.clone(), token.clone()))
        } else {
            Arc::new(InMemoryVectorStore::new())
        };

        let blob: Arc<dyn BlobStorePort> = if let Some(token) = &config.upstash_blob_token {
            Arc::new(UpstashBlobStore::new(token.clone()))
        } else {
            Arc::new(LocalFsBlobStore::new(std::path::PathBuf::from(
                "./data/blobs",
            )))
        };

        Ok(Self {
            mode,
            state,
            bus,
            vector,
            blob,
        })
    }

    /// Creates an in-memory hermetic registry for integration tests.
    #[must_use]
    pub fn in_memory() -> Self {
        Self {
            mode: InfrastructureMode::SelfHosted,
            state: InMemoryStateBackend::new(),
            bus: InMemoryMessageBus::new(),
            vector: Arc::new(InMemoryVectorStore::new()),
            blob: Arc::new(LocalFsBlobStore::new(std::path::PathBuf::from(
                "./target/test-blobs",
            ))),
        }
    }
}
