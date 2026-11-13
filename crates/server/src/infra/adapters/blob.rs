//! Hexagonal BlobStorePort and adapters for file storage and artifact blobs.

use std::path::PathBuf;

use tokio::fs;

use crate::infra::{error::ServerError, state::BoxFuture};

/// Abstract port for artifact and file blob persistence.
pub trait BlobStorePort: Send + Sync + 'static {
    fn put_blob<'a>(
        &'a self,
        key: &'a str,
        data: Vec<u8>,
        content_type: &'a str,
    ) -> BoxFuture<'a, Result<String, ServerError>>;

    fn get_blob<'a>(&'a self, key: &'a str) -> BoxFuture<'a, Result<Vec<u8>, ServerError>>;

    fn delete_blob<'a>(&'a self, key: &'a str) -> BoxFuture<'a, Result<(), ServerError>>;
}

/// Local filesystem blob store for sovereign self-hosted mode and hermetic tests.
#[derive(Clone)]
pub struct LocalFsBlobStore {
    root_dir: PathBuf,
}

impl LocalFsBlobStore {
    #[must_use]
    pub fn new(root_dir: PathBuf) -> Self {
        Self { root_dir }
    }

    fn resolve_path(&self, key: &str) -> Result<PathBuf, ServerError> {
        let clean = key.trim_start_matches('/');
        if clean.contains("..") || clean.contains('\\') {
            return Err(ServerError::Validation("invalid blob key traversal".into()));
        }
        Ok(self.root_dir.join(clean))
    }
}

impl BlobStorePort for LocalFsBlobStore {
    fn put_blob<'a>(
        &'a self,
        key: &'a str,
        data: Vec<u8>,
        _content_type: &'a str,
    ) -> BoxFuture<'a, Result<String, ServerError>> {
        Box::pin(async move {
            let path = self.resolve_path(key)?;
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent).await.map_err(|e| {
                    ServerError::Internal(format!("failed to create blob dir: {e}"))
                })?;
            }
            fs::write(&path, &data)
                .await
                .map_err(|e| ServerError::Internal(format!("failed to write blob: {e}")))?;
            Ok(key.to_owned())
        })
    }

    fn get_blob<'a>(&'a self, key: &'a str) -> BoxFuture<'a, Result<Vec<u8>, ServerError>> {
        Box::pin(async move {
            let path = self.resolve_path(key)?;
            fs::read(&path)
                .await
                .map_err(|e| ServerError::NotFound(format!("blob `{key}` not found: {e}")))
        })
    }

    fn delete_blob<'a>(&'a self, key: &'a str) -> BoxFuture<'a, Result<(), ServerError>> {
        Box::pin(async move {
            let path = self.resolve_path(key)?;
            if path.exists() {
                fs::remove_file(&path)
                    .await
                    .map_err(|e| ServerError::Internal(format!("failed to remove blob: {e}")))?;
            }
            Ok(())
        })
    }
}

/// Upstash Blob REST adapter implementing [`BlobStorePort`].
#[derive(Clone)]
pub struct UpstashBlobStore {
    endpoint: String,
    token: String,
    client: reqwest::Client,
}

impl UpstashBlobStore {
    #[must_use]
    pub fn new(token: String) -> Self {
        Self {
            endpoint: "https://blob.upstash.com".to_owned(),
            token,
            client: reqwest::Client::new(),
        }
    }

    #[must_use]
    pub fn with_endpoint(endpoint: String, token: String) -> Self {
        Self {
            endpoint: endpoint.trim_end_matches('/').to_owned(),
            token,
            client: reqwest::Client::new(),
        }
    }
}

impl BlobStorePort for UpstashBlobStore {
    fn put_blob<'a>(
        &'a self,
        key: &'a str,
        data: Vec<u8>,
        content_type: &'a str,
    ) -> BoxFuture<'a, Result<String, ServerError>> {
        Box::pin(async move {
            let clean_key = key.trim_start_matches('/');
            let url = format!("{}/{}", self.endpoint, clean_key);
            let res = self
                .client
                .put(&url)
                .bearer_auth(&self.token)
                .header(reqwest::header::CONTENT_TYPE, content_type)
                .body(data)
                .send()
                .await
                .map_err(|e| ServerError::Internal(format!("upstash blob put network: {e}")))?;
            if !res.status().is_success() {
                let status = res.status();
                let txt = res.text().await.unwrap_or_default();
                return Err(ServerError::Internal(format!(
                    "upstash blob put failed {status}: {txt}"
                )));
            }
            Ok(url)
        })
    }

    fn get_blob<'a>(&'a self, key: &'a str) -> BoxFuture<'a, Result<Vec<u8>, ServerError>> {
        Box::pin(async move {
            let clean_key = key.trim_start_matches('/');
            let url = format!("{}/{}", self.endpoint, clean_key);
            let res = self
                .client
                .get(&url)
                .bearer_auth(&self.token)
                .send()
                .await
                .map_err(|e| ServerError::Internal(format!("upstash blob get network: {e}")))?;
            if !res.status().is_success() {
                let status = res.status();
                let txt = res.text().await.unwrap_or_default();
                return Err(ServerError::NotFound(format!(
                    "upstash blob get failed {status}: {txt}"
                )));
            }
            let bytes = res
                .bytes()
                .await
                .map_err(|e| ServerError::Internal(format!("upstash blob read bytes: {e}")))?;
            Ok(bytes.to_vec())
        })
    }

    fn delete_blob<'a>(&'a self, key: &'a str) -> BoxFuture<'a, Result<(), ServerError>> {
        Box::pin(async move {
            let clean_key = key.trim_start_matches('/');
            let url = format!("{}/{}", self.endpoint, clean_key);
            let res = self
                .client
                .delete(&url)
                .bearer_auth(&self.token)
                .send()
                .await
                .map_err(|e| ServerError::Internal(format!("upstash blob delete network: {e}")))?;
            if !res.status().is_success() {
                let status = res.status();
                let txt = res.text().await.unwrap_or_default();
                return Err(ServerError::Internal(format!(
                    "upstash blob delete failed {status}: {txt}"
                )));
            }
            Ok(())
        })
    }
}
