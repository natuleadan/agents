use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::path::PathBuf;
use std::sync::{Arc, RwLock};

use agents_protocol::{EventEnvelope, StreamPayload};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::ClientError;
use super::backend::{BackendTarget, ControlAction};

/// Persistent secret storage contract (in-memory for WASM/tests, scoped file/keychain on native).
pub trait SecretStore: Send + Sync {
    fn store_secret(&self, account: &str, secret: &str) -> Result<(), ClientError>;
    fn load_secret(&self, account: &str) -> Result<Option<String>, ClientError>;
    fn delete_secret(&self, account: &str) -> Result<(), ClientError>;
    fn list_accounts(&self) -> Result<Vec<String>, ClientError>;
}

/// Thread-safe in-memory secret store that redacts all secret values in Debug output.
#[derive(Clone, Default)]
pub struct InMemorySecretStore {
    entries: Arc<RwLock<BTreeMap<String, String>>>,
}

impl fmt::Debug for InMemorySecretStore {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let count = self.entries.read().map(|m| m.len()).unwrap_or(0);
        f.debug_struct("InMemorySecretStore")
            .field("entry_count", &count)
            .field("secrets", &"[REDACTED]")
            .finish()
    }
}

impl SecretStore for InMemorySecretStore {
    fn store_secret(&self, account: &str, secret: &str) -> Result<(), ClientError> {
        let mut guard = self
            .entries
            .write()
            .map_err(|_| ClientError::Storage("secret lock poisoned".to_owned()))?;
        guard.insert(account.to_owned(), secret.to_owned());
        Ok(())
    }

    fn load_secret(&self, account: &str) -> Result<Option<String>, ClientError> {
        let guard = self
            .entries
            .read()
            .map_err(|_| ClientError::Storage("secret lock poisoned".to_owned()))?;
        Ok(guard.get(account).cloned())
    }

    fn delete_secret(&self, account: &str) -> Result<(), ClientError> {
        let mut guard = self
            .entries
            .write()
            .map_err(|_| ClientError::Storage("secret lock poisoned".to_owned()))?;
        guard.remove(account);
        Ok(())
    }

    fn list_accounts(&self) -> Result<Vec<String>, ClientError> {
        let guard = self
            .entries
            .read()
            .map_err(|_| ClientError::Storage("secret lock poisoned".to_owned()))?;
        Ok(guard.keys().cloned().collect())
    }
}

/// Scoped keychain-compatible store writing hashed-index entries on native hosts with fallback.
#[derive(Clone)]
pub struct ScopedKeychainStore {
    service: String,
    directory: Option<PathBuf>,
    fallback: InMemorySecretStore,
}

impl fmt::Debug for ScopedKeychainStore {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ScopedKeychainStore")
            .field("service", &self.service)
            .field("directory", &self.directory)
            .field("secrets", &"[REDACTED]")
            .finish()
    }
}

impl ScopedKeychainStore {
    #[must_use]
    pub fn new(service: impl Into<String>, directory: Option<PathBuf>) -> Self {
        Self {
            service: service.into(),
            directory,
            fallback: InMemorySecretStore::default(),
        }
    }

    fn account_path(&self, account: &str) -> Option<PathBuf> {
        let dir = self.directory.as_ref()?;
        let mut hasher = Sha256::new();
        hasher.update(self.service.as_bytes());
        hasher.update(b":");
        hasher.update(account.as_bytes());
        let name = format!("{}.key", hex::encode(hasher.finalize()));
        Some(dir.join(name))
    }
}

impl SecretStore for ScopedKeychainStore {
    fn store_secret(&self, account: &str, secret: &str) -> Result<(), ClientError> {
        self.fallback.store_secret(account, secret)?;
        #[cfg(not(target_arch = "wasm32"))]
        if let Some(path) = self.account_path(account) {
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent)
                    .map_err(|e| ClientError::Storage(format!("create keychain dir: {e}")))?;
            }
            std::fs::write(&path, secret.as_bytes())
                .map_err(|e| ClientError::Storage(format!("write keychain file: {e}")))?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let perms = std::fs::Permissions::from_mode(0o600);
                let _ = std::fs::set_permissions(&path, perms);
            }
        }
        Ok(())
    }

    fn load_secret(&self, account: &str) -> Result<Option<String>, ClientError> {
        if let Some(cached) = self.fallback.load_secret(account)? {
            return Ok(Some(cached));
        }
        #[cfg(not(target_arch = "wasm32"))]
        if let Some(path) = self.account_path(account)
            && path.exists()
        {
            let value = std::fs::read_to_string(&path)
                .map_err(|e| ClientError::Storage(format!("read keychain file: {e}")))?;
            let _ = self.fallback.store_secret(account, &value);
            return Ok(Some(value));
        }
        Ok(None)
    }

    fn delete_secret(&self, account: &str) -> Result<(), ClientError> {
        self.fallback.delete_secret(account)?;
        #[cfg(not(target_arch = "wasm32"))]
        if let Some(path) = self.account_path(account)
            && path.exists()
        {
            std::fs::remove_file(&path)
                .map_err(|e| ClientError::Storage(format!("delete keychain file: {e}")))?;
        }
        Ok(())
    }

    fn list_accounts(&self) -> Result<Vec<String>, ClientError> {
        self.fallback.list_accounts()
    }
}

/// Offline profile snapshot enabling cold boot when the remote server is unreachable.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OfflineStartupState {
    pub profile_id: String,
    pub target: BackendTarget,
    pub last_session_id: Option<String>,
    pub model_name: String,
}

impl Default for OfflineStartupState {
    fn default() -> Self {
        Self {
            profile_id: "default-local".to_owned(),
            target: BackendTarget::local_server("http://127.0.0.1:8080"),
            last_session_id: None,
            model_name: "claude-sonnet-4-5".to_owned(),
        }
    }
}

/// Persisted composer draft supporting local crash recovery across disconnects.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DraftRecord {
    pub session_id: String,
    pub prompt: String,
    pub attachments: Vec<String>,
    pub updated_at_ms: u64,
}

/// Local draft & last-profile store.
#[derive(Debug, Clone, Default)]
pub struct DraftStore {
    drafts: Arc<RwLock<BTreeMap<String, DraftRecord>>>,
    startup: Arc<RwLock<OfflineStartupState>>,
}

impl DraftStore {
    pub fn save_draft(&self, record: DraftRecord) -> Result<(), ClientError> {
        let mut guard = self
            .drafts
            .write()
            .map_err(|_| ClientError::Storage("draft lock poisoned".to_owned()))?;
        guard.insert(record.session_id.clone(), record);
        Ok(())
    }

    pub fn load_draft(&self, session_id: &str) -> Result<Option<DraftRecord>, ClientError> {
        let guard = self
            .drafts
            .read()
            .map_err(|_| ClientError::Storage("draft lock poisoned".to_owned()))?;
        Ok(guard.get(session_id).cloned())
    }

    pub fn clear_draft(&self, session_id: &str) -> Result<(), ClientError> {
        let mut guard = self
            .drafts
            .write()
            .map_err(|_| ClientError::Storage("draft lock poisoned".to_owned()))?;
        guard.remove(session_id);
        Ok(())
    }

    pub fn save_startup_profile(&self, state: OfflineStartupState) -> Result<(), ClientError> {
        let mut guard = self
            .startup
            .write()
            .map_err(|_| ClientError::Storage("startup profile lock poisoned".to_owned()))?;
        *guard = state;
        Ok(())
    }

    pub fn load_startup_profile(&self) -> Result<OfflineStartupState, ClientError> {
        let guard = self
            .startup
            .read()
            .map_err(|_| ClientError::Storage("startup profile lock poisoned".to_owned()))?;
        Ok(guard.clone())
    }
}

/// Queued outbound control operation carrying a deterministic `Idempotency-Key`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct QueuedAction {
    pub task_id: String,
    pub action: ControlAction,
    pub idempotency_key: String,
    pub attempts: u32,
}

#[derive(Debug, Clone, Default)]
pub struct IdempotentActionQueue {
    counter: u64,
    pending: Vec<QueuedAction>,
    settled_keys: BTreeSet<String>,
}

impl IdempotentActionQueue {
    pub fn enqueue(&mut self, task_id: impl Into<String>, action: ControlAction) -> QueuedAction {
        let task_id = task_id.into();
        self.counter += 1;
        let key = format!("{task_id}:{}:{}", action.as_str(), self.counter);
        let item = QueuedAction {
            task_id,
            action,
            idempotency_key: key,
            attempts: 0,
        };
        self.pending.push(item.clone());
        item
    }

    #[must_use]
    pub fn pending(&self) -> &[QueuedAction] {
        &self.pending
    }

    pub fn mark_attempt(&mut self, idempotency_key: &str) {
        if let Some(item) = self
            .pending
            .iter_mut()
            .find(|item| item.idempotency_key == idempotency_key)
        {
            item.attempts += 1;
        }
    }

    pub fn acknowledge(&mut self, idempotency_key: &str) {
        self.settled_keys.insert(idempotency_key.to_owned());
        self.pending
            .retain(|item| item.idempotency_key != idempotency_key);
    }

    #[must_use]
    pub fn is_settled(&self, idempotency_key: &str) -> bool {
        self.settled_keys.contains(idempotency_key)
    }
}

/// Result returned by [`ChunkCoalescer::ingest`] for ordered stream processing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CoalesceOutcome {
    Appended {
        seq: u64,
        chunk: Option<String>,
    },
    DuplicateIgnored {
        seq: u64,
    },
    GapDetected {
        expected_seq: u64,
        received_seq: u64,
    },
}

/// Buffers out-of-order frames, deduplicates replays, and tracks gaps for WebSocket reconnect.
#[derive(Debug, Clone, Default)]
pub struct ChunkCoalescer {
    last_applied_seq: u64,
    buffer: BTreeMap<u64, EventEnvelope>,
    merged_output: String,
}

impl ChunkCoalescer {
    #[must_use]
    pub fn last_applied_seq(&self) -> u64 {
        self.last_applied_seq
    }

    #[must_use]
    pub fn merged_output(&self) -> &str {
        &self.merged_output
    }

    pub fn ingest(&mut self, envelope: EventEnvelope) -> Vec<CoalesceOutcome> {
        let seq = envelope.seq;
        if seq <= self.last_applied_seq || self.buffer.contains_key(&seq) {
            return vec![CoalesceOutcome::DuplicateIgnored { seq }];
        }

        self.buffer.insert(seq, envelope);
        let expected = self.last_applied_seq + 1;
        if seq > expected {
            return vec![CoalesceOutcome::GapDetected {
                expected_seq: expected,
                received_seq: seq,
            }];
        }

        self.drain_contiguous()
    }

    fn drain_contiguous(&mut self) -> Vec<CoalesceOutcome> {
        let mut results = Vec::new();
        while let Some(next_seq) = self.buffer.keys().next().copied() {
            if next_seq != self.last_applied_seq + 1 {
                break;
            }
            if let Some(env) = self.buffer.remove(&next_seq) {
                self.last_applied_seq = next_seq;
                let chunk = match &env.payload {
                    StreamPayload::TaskChunk { chunk, .. } => {
                        self.merged_output.push_str(chunk);
                        Some(chunk.clone())
                    }
                    _ => None,
                };
                results.push(CoalesceOutcome::Appended {
                    seq: next_seq,
                    chunk,
                });
            }
        }
        results
    }
}
