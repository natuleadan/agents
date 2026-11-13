//! Hexagonal VectorStorePort and adapters for semantic memory and vector indexing.

use std::{collections::HashMap, sync::Arc};

use serde::{Deserialize, Serialize};
use tokio::sync::RwLock;

use crate::infra::{error::ServerError, state::BoxFuture};

/// Scored vector search result match.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VectorMatch {
    pub id: String,
    pub score: f32,
    pub metadata: Option<serde_json::Value>,
}

/// Abstract port for vector embeddings storage and semantic similarity queries.
pub trait VectorStorePort: Send + Sync + 'static {
    fn upsert<'a>(
        &'a self,
        id: &'a str,
        vector: &'a [f32],
        metadata: Option<serde_json::Value>,
    ) -> BoxFuture<'a, Result<(), ServerError>>;

    fn query<'a>(
        &'a self,
        vector: &'a [f32],
        top_k: usize,
    ) -> BoxFuture<'a, Result<Vec<VectorMatch>, ServerError>>;

    fn delete<'a>(&'a self, id: &'a str) -> BoxFuture<'a, Result<(), ServerError>>;
}

type StoredVectorMap = HashMap<String, (Vec<f32>, Option<serde_json::Value>)>;

/// In-memory vector store performing cosine similarity search for tests and sovereign mode.
#[derive(Clone, Default)]
pub struct InMemoryVectorStore {
    vectors: Arc<RwLock<StoredVectorMap>>,
}

impl InMemoryVectorStore {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }
}

fn cosine_similarity(a: &[f32], b: &[f32]) -> f32 {
    if a.len() != b.len() || a.is_empty() {
        return 0.0;
    }
    let mut dot = 0.0_f32;
    let mut norm_a = 0.0_f32;
    let mut norm_b = 0.0_f32;
    for (x, y) in a.iter().zip(b.iter()) {
        dot += x * y;
        norm_a += x * x;
        norm_b += y * y;
    }
    if norm_a <= 0.0 || norm_b <= 0.0 {
        0.0
    } else {
        dot / (norm_a.sqrt() * norm_b.sqrt())
    }
}

impl VectorStorePort for InMemoryVectorStore {
    fn upsert<'a>(
        &'a self,
        id: &'a str,
        vector: &'a [f32],
        metadata: Option<serde_json::Value>,
    ) -> BoxFuture<'a, Result<(), ServerError>> {
        Box::pin(async move {
            let mut store = self.vectors.write().await;
            store.insert(id.to_owned(), (vector.to_vec(), metadata));
            Ok(())
        })
    }

    fn query<'a>(
        &'a self,
        vector: &'a [f32],
        top_k: usize,
    ) -> BoxFuture<'a, Result<Vec<VectorMatch>, ServerError>> {
        Box::pin(async move {
            let store = self.vectors.read().await;
            let mut scored: Vec<VectorMatch> = store
                .iter()
                .map(|(id, (stored_vec, meta))| {
                    let score = cosine_similarity(vector, stored_vec);
                    VectorMatch {
                        id: id.clone(),
                        score,
                        metadata: meta.clone(),
                    }
                })
                .collect();
            scored.sort_by(|a, b| {
                b.score
                    .partial_cmp(&a.score)
                    .unwrap_or(std::cmp::Ordering::Equal)
            });
            scored.truncate(top_k);
            Ok(scored)
        })
    }

    fn delete<'a>(&'a self, id: &'a str) -> BoxFuture<'a, Result<(), ServerError>> {
        Box::pin(async move {
            let mut store = self.vectors.write().await;
            store.remove(id);
            Ok(())
        })
    }
}

/// Upstash Vector REST adapter implementing [`VectorStorePort`].
#[derive(Clone)]
pub struct UpstashVectorAdapter {
    url: String,
    token: String,
    client: reqwest::Client,
}

impl UpstashVectorAdapter {
    #[must_use]
    pub fn new(url: String, token: String) -> Self {
        Self {
            url: url.trim_end_matches('/').to_owned(),
            token,
            client: reqwest::Client::new(),
        }
    }
}

#[derive(Serialize)]
struct UpstashUpsertReq<'a> {
    id: &'a str,
    vector: &'a [f32],
    #[serde(skip_serializing_if = "Option::is_none")]
    metadata: Option<serde_json::Value>,
}

#[derive(Serialize)]
struct UpstashQueryReq<'a> {
    vector: &'a [f32],
    #[serde(rename = "topK")]
    top_k: usize,
    #[serde(rename = "includeMetadata")]
    include_metadata: bool,
}

#[derive(Deserialize)]
struct UpstashQueryResultItem {
    id: String,
    score: f32,
    metadata: Option<serde_json::Value>,
}

impl VectorStorePort for UpstashVectorAdapter {
    fn upsert<'a>(
        &'a self,
        id: &'a str,
        vector: &'a [f32],
        metadata: Option<serde_json::Value>,
    ) -> BoxFuture<'a, Result<(), ServerError>> {
        Box::pin(async move {
            let payload = UpstashUpsertReq {
                id,
                vector,
                metadata,
            };
            let endpoint = format!("{}/upsert", self.url);
            let res = self
                .client
                .post(&endpoint)
                .bearer_auth(&self.token)
                .json(&payload)
                .send()
                .await
                .map_err(|e| {
                    ServerError::Internal(format!("upstash vector upsert network: {e}"))
                })?;
            if !res.status().is_success() {
                let status = res.status();
                let txt = res.text().await.unwrap_or_default();
                return Err(ServerError::Internal(format!(
                    "upstash vector upsert failed {status}: {txt}"
                )));
            }
            Ok(())
        })
    }

    fn query<'a>(
        &'a self,
        vector: &'a [f32],
        top_k: usize,
    ) -> BoxFuture<'a, Result<Vec<VectorMatch>, ServerError>> {
        Box::pin(async move {
            let payload = UpstashQueryReq {
                vector,
                top_k,
                include_metadata: true,
            };
            let endpoint = format!("{}/query", self.url);
            let res = self
                .client
                .post(&endpoint)
                .bearer_auth(&self.token)
                .json(&payload)
                .send()
                .await
                .map_err(|e| ServerError::Internal(format!("upstash vector query network: {e}")))?;
            if !res.status().is_success() {
                let status = res.status();
                let txt = res.text().await.unwrap_or_default();
                return Err(ServerError::Internal(format!(
                    "upstash vector query failed {status}: {txt}"
                )));
            }
            let raw: Vec<UpstashQueryResultItem> = res
                .json()
                .await
                .map_err(|e| ServerError::Internal(format!("upstash vector parse error: {e}")))?;
            let matches = raw
                .into_iter()
                .map(|item| VectorMatch {
                    id: item.id,
                    score: item.score,
                    metadata: item.metadata,
                })
                .collect();
            Ok(matches)
        })
    }

    fn delete<'a>(&'a self, id: &'a str) -> BoxFuture<'a, Result<(), ServerError>> {
        Box::pin(async move {
            let endpoint = format!("{}/delete", self.url);
            let res = self
                .client
                .post(&endpoint)
                .bearer_auth(&self.token)
                .json(&serde_json::json!({ "ids": [id] }))
                .send()
                .await
                .map_err(|e| {
                    ServerError::Internal(format!("upstash vector delete network: {e}"))
                })?;
            if !res.status().is_success() {
                let status = res.status();
                let txt = res.text().await.unwrap_or_default();
                return Err(ServerError::Internal(format!(
                    "upstash vector delete failed {status}: {txt}"
                )));
            }
            Ok(())
        })
    }
}
