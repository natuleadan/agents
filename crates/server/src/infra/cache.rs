//! Asynchronous cache wrapper backed by native Ironic cache contracts.

use std::{sync::Arc, time::Duration};

use ironic::services::cache::{Cache, InMemoryCache};
use serde::{Serialize, de::DeserializeOwned};

/// Injectable application cache wrapper backed by [`ironic::services::cache::Cache`].
#[derive(Clone)]
pub struct AppCache {
    backend: Arc<dyn Cache>,
}

impl AppCache {
    /// Constructs an [`AppCache`] wrapping any [`Cache`] implementation.
    #[must_use]
    pub fn new(backend: Arc<dyn Cache>) -> Self {
        Self { backend }
    }

    /// Constructs an in-memory bounded cache with the given entry capacity.
    #[must_use]
    pub fn in_memory(capacity: usize) -> Self {
        Self {
            backend: Arc::new(InMemoryCache::new(capacity)),
        }
    }

    /// Loads and deserializes JSON from cache. Returns `None` on miss or error.
    pub async fn get_json<T: DeserializeOwned>(&self, key: &str) -> Option<T> {
        let bytes = self.backend.get(key).await.ok()??;
        serde_json::from_slice(&bytes).ok()
    }

    /// Serializes and stores JSON with a time-to-live.
    pub async fn set_json<T: Serialize>(&self, key: &str, value: &T, ttl: Duration) {
        if let Ok(bytes) = serde_json::to_vec(value) {
            let _ = self.backend.set(key, bytes, Some(ttl)).await;
        }
    }

    /// Evicts a specific key from the cache.
    pub async fn remove(&self, key: &str) -> bool {
        self.backend.remove(key).await.unwrap_or(false)
    }

    /// Evicts all keys starting with the given prefix.
    pub async fn remove_by_prefix(&self, prefix: &str) -> usize {
        self.backend.remove_by_prefix(prefix).await.unwrap_or(0)
    }
}

impl Default for AppCache {
    fn default() -> Self {
        Self::in_memory(4096)
    }
}
