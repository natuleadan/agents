//! Tamper-evident hash-chained audit trail service and retention archival logic.

use std::sync::Arc;

use ironic::prelude::*;
use serde_json::Value;
use sha2::{Digest, Sha256};
use tokio::sync::Mutex;

use crate::{
    infra::{config::ServerConfig, error::ServerError, state::now_ms},
    modules::audit::{
        dto::{AuditQueryDto, AuditVerificationReport},
        entities::AuditLogRecord,
        repositories::{AuditStore, GENESIS_AUDIT_HASH},
    },
};

/// Records security events chained via SHA-256 digests and enforces log retention.
#[derive(Injectable)]
pub struct AuditService {
    store: Arc<AuditStore>,
    config: Arc<ServerConfig>,
}

static APPEND_CHAIN_LOCK: std::sync::LazyLock<Mutex<()>> =
    std::sync::LazyLock::new(|| Mutex::new(()));

impl AuditService {
    /// Appends a tamper-evident audit entry chained to the previous digest.
    ///
    /// # Errors
    ///
    /// Returns [`ServerError`] when persistence fails.
    pub async fn record(
        &self,
        actor_id: Option<&str>,
        action: &str,
        resource_type: &str,
        resource_id: Option<&str>,
        outcome: &str,
        details: Value,
    ) -> Result<AuditLogRecord, ServerError> {
        let _guard = APPEND_CHAIN_LOCK.lock().await;
        let prev_hash = self.store.0.last_hash().await?;
        let id = uuid::Uuid::new_v4().to_string();
        let created_at = now_ms();
        let entry_hash = compute_entry_hash(
            &prev_hash,
            &id,
            actor_id,
            action,
            resource_type,
            resource_id,
            outcome,
            &details,
            created_at,
        );
        let entry = AuditLogRecord {
            id,
            actor_id: actor_id.map(ToOwned::to_owned),
            action: action.to_owned(),
            resource_type: resource_type.to_owned(),
            resource_id: resource_id.map(ToOwned::to_owned),
            outcome: outcome.to_owned(),
            details,
            prev_hash,
            entry_hash,
            created_at,
        };
        self.store.0.append(entry.clone()).await?;
        Ok(entry)
    }

    /// Queries recent audit entries matching optional actor/action filters.
    ///
    /// # Errors
    ///
    /// Returns [`ServerError`] when the backing repository fails.
    pub async fn list(&self, query: &AuditQueryDto) -> Result<Vec<AuditLogRecord>, ServerError> {
        let limit = query.limit.unwrap_or(50);
        self.store
            .0
            .list(query.actor_id.as_deref(), query.action.as_deref(), limit)
            .await
    }

    /// Recomputes every entry hash chronologically to detect tampering.
    ///
    /// # Errors
    ///
    /// Returns [`ServerError`] when reading entries fails.
    pub async fn verify_chain(&self) -> Result<AuditVerificationReport, ServerError> {
        let entries = self.store.0.list_chronological().await?;
        let mut expected_prev = GENESIS_AUDIT_HASH.to_owned();
        for entry in &entries {
            if entry.prev_hash != expected_prev {
                return Ok(AuditVerificationReport {
                    checked_entries: entries.len(),
                    valid: false,
                    broken_entry_id: Some(entry.id.clone()),
                });
            }
            let recomputed = compute_entry_hash(
                &entry.prev_hash,
                &entry.id,
                entry.actor_id.as_deref(),
                &entry.action,
                &entry.resource_type,
                entry.resource_id.as_deref(),
                &entry.outcome,
                &entry.details,
                entry.created_at,
            );
            if recomputed != entry.entry_hash {
                return Ok(AuditVerificationReport {
                    checked_entries: entries.len(),
                    valid: false,
                    broken_entry_id: Some(entry.id.clone()),
                });
            }
            expected_prev = entry.entry_hash.clone();
        }
        Ok(AuditVerificationReport {
            checked_entries: entries.len(),
            valid: true,
            broken_entry_id: None,
        })
    }

    /// Prunes entries older than the configured retention window (`audit_retention_secs`).
    ///
    /// # Errors
    ///
    /// Returns [`ServerError`] when pruning fails.
    pub async fn prune_expired(&self) -> Result<u64, ServerError> {
        let retention_ms = self.config.audit_retention_secs.saturating_mul(1_000);
        let cutoff = now_ms().saturating_sub(retention_ms);
        self.store.0.purge_older_than(cutoff).await
    }
}

#[allow(clippy::too_many_arguments)]
#[must_use]
pub fn compute_entry_hash(
    prev_hash: &str,
    id: &str,
    actor_id: Option<&str>,
    action: &str,
    resource_type: &str,
    resource_id: Option<&str>,
    outcome: &str,
    details: &Value,
    created_at: u64,
) -> String {
    let mut hasher = Sha256::new();
    hasher.update(prev_hash.as_bytes());
    hasher.update(b"|");
    hasher.update(id.as_bytes());
    hasher.update(b"|");
    hasher.update(actor_id.unwrap_or("").as_bytes());
    hasher.update(b"|");
    hasher.update(action.as_bytes());
    hasher.update(b"|");
    hasher.update(resource_type.as_bytes());
    hasher.update(b"|");
    hasher.update(resource_id.unwrap_or("").as_bytes());
    hasher.update(b"|");
    hasher.update(outcome.as_bytes());
    hasher.update(b"|");
    hasher.update(details.to_string().as_bytes());
    hasher.update(b"|");
    hasher.update(created_at.to_be_bytes());
    hex::encode(hasher.finalize())
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;
    use crate::modules::audit::repositories::{AuditRepository, InMemoryAuditRepository};

    #[tokio::test]
    async fn verify_chain_detects_tampering_and_prune_enforces_retention() {
        let repo = InMemoryAuditRepository::new();
        let mut cfg = ServerConfig::for_tests();
        cfg.audit_retention_secs = 0;

        let service = AuditService {
            store: Arc::new(AuditStore(Arc::clone(&repo) as Arc<dyn AuditRepository>)),
            config: Arc::new(cfg),
        };

        let first = service
            .record(
                Some("usr-1"),
                "auth.login",
                "user",
                Some("usr-1"),
                "ok",
                serde_json::json!({}),
            )
            .await
            .unwrap();
        let second = service
            .record(
                Some("usr-1"),
                "session.create",
                "session",
                Some("s-1"),
                "ok",
                serde_json::json!({}),
            )
            .await
            .unwrap();
        assert_eq!(second.prev_hash, first.entry_hash);

        let clean = service.verify_chain().await.unwrap();
        assert!(clean.valid);
        assert_eq!(clean.checked_entries, 2);

        // Append an out-of-chain tampered record and verify verify_chain catches it.
        repo.append(AuditLogRecord {
            id: "tampered-entry".to_owned(),
            actor_id: None,
            action: "evil".to_owned(),
            resource_type: "audit".to_owned(),
            resource_id: None,
            outcome: "ok".to_owned(),
            details: serde_json::json!({}),
            prev_hash: "deadbeef".to_owned(),
            entry_hash: "badhash".to_owned(),
            created_at: now_ms(),
        })
        .await
        .unwrap();

        let broken = service.verify_chain().await.unwrap();
        assert!(!broken.valid);
        assert_eq!(broken.broken_entry_id.as_deref(), Some("tampered-entry"));

        // With 0s retention, purging below future cutoff clears expired records.
        let purged = repo.purge_older_than(u64::MAX).await.unwrap();
        assert_eq!(purged, 3);
    }
}
