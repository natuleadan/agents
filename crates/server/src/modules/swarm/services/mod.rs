//! Swarm coordination service enforcing OWASP ASI-05 message signatures,
//! OWASP ASI-07 child permission narrowing, Valkey blackboard sharing, and NATS fanout.

use std::{sync::Arc, time::Duration};

use agents_protocol::{SwarmMessage, SwarmTopic};
use ironic::prelude::*;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;

use crate::{
    infra::{config::ServerConfig, error::ServerError, state::now_ms, stream::BusClient},
    modules::{
        audit::services::AuditService,
        auth::{entities::AuthenticatedActor, services::StateStore},
        sessions::services::SessionService,
        swarm::{
            dto::{CreateHandoffDto, PutBlackboardDto, SendSwarmMessageDto},
            entities::{MediatedSwarmMessage, SwarmHandoffEntity},
            repositories::SwarmStore,
        },
    },
};

/// Coordinates multi-agent handoffs, NATS swarm topics, and Valkey shared blackboards.
#[derive(Injectable)]
pub struct SwarmService {
    store: Arc<SwarmStore>,
    sessions: Arc<SessionService>,
    state: Arc<StateStore>,
    bus: Arc<BusClient>,
    config: Arc<ServerConfig>,
    audit: Arc<AuditService>,
}

impl SwarmService {
    /// Validates OWASP ASI-07 child subset narrowing and persists a subagent handoff.
    ///
    /// # Errors
    ///
    /// Returns [`ServerError::Forbidden`] if session is unowned or permission escalation is attempted.
    pub async fn create_handoff(
        &self,
        actor: &AuthenticatedActor,
        session_id: &str,
        dto: &CreateHandoffDto,
    ) -> Result<SwarmHandoffEntity, ServerError> {
        let _ = self.sessions.get_owned(actor, session_id).await?;

        let delegated_mask = match dto.parent_mask.narrow(&dto.requested_mask) {
            Ok(mask) => mask,
            Err(err) => {
                let _ = self
                    .audit
                    .record(
                        Some(&actor.user_id),
                        "swarm.handoff",
                        "session",
                        Some(session_id),
                        "denied_escalation",
                        json!({
                            "parentAgentId": dto.parent_agent_id,
                            "childAgentId": dto.child_agent_id,
                            "reason": err.to_string(),
                        }),
                    )
                    .await;
                return Err(ServerError::Forbidden(err.to_string()));
            }
        };

        let entity = SwarmHandoffEntity {
            handoff_id: uuid::Uuid::new_v4().to_string(),
            session_id: session_id.to_owned(),
            owner_id: actor.user_id.clone(),
            parent_agent_id: dto.parent_agent_id.clone(),
            child_agent_id: dto.child_agent_id.clone(),
            worktree_branch: dto.worktree_branch.clone(),
            delegated_mask,
            goal: dto.goal.clone(),
            created_at: now_ms(),
        };

        let saved = self.store.0.save_handoff(entity).await?;
        let _ = self
            .audit
            .record(
                Some(&actor.user_id),
                "swarm.handoff",
                "handoff",
                Some(&saved.handoff_id),
                "success",
                json!({
                    "sessionId": session_id,
                    "childAgentId": saved.child_agent_id,
                }),
            )
            .await;

        Ok(saved)
    }

    /// Lists recorded handoffs for an owned session.
    ///
    /// # Errors
    ///
    /// Returns [`ServerError::Forbidden`] or [`ServerError::NotFound`].
    pub async fn list_handoffs(
        &self,
        actor: &AuthenticatedActor,
        session_id: &str,
    ) -> Result<Vec<SwarmHandoffEntity>, ServerError> {
        let _ = self.sessions.get_owned(actor, session_id).await?;
        self.store.0.list_handoffs(session_id).await
    }

    /// Dispatches a signed OWASP ASI-05 message across `swarm.<session>.*` NATS topics.
    ///
    /// If the caller provides an explicit `signature`, it is verified using constant-time HMAC-SHA256.
    ///
    /// # Errors
    ///
    /// Returns [`ServerError::Forbidden`] when signature verification fails or session is unowned.
    pub async fn send_message(
        &self,
        actor: &AuthenticatedActor,
        session_id: &str,
        dto: &SendSwarmMessageDto,
    ) -> Result<MediatedSwarmMessage, ServerError> {
        let _ = self.sessions.get_owned(actor, session_id).await?;

        let topic = match dto.target_agent_id.as_deref().map(str::trim) {
            Some(target) if !target.is_empty() => SwarmTopic::DirectAgent(target.to_owned()),
            _ => SwarmTopic::Broadcast,
        };

        let mut wire = SwarmMessage {
            message_id: uuid::Uuid::new_v4().to_string(),
            session_id: session_id.to_owned(),
            from_agent_id: dto.from_agent_id.clone(),
            to: topic,
            payload: dto.payload.clone(),
            permission_mask: dto.permission_mask.clone(),
            signature: String::new(),
            emitted_at_ms: now_ms(),
        };

        let computed_sig = self.sign_swarm_message(&wire);
        if let Some(provided) = dto.signature.as_deref().filter(|s| !s.is_empty()) {
            let matches = computed_sig
                .as_bytes()
                .ct_eq(provided.as_bytes())
                .unwrap_u8()
                == 1;
            if !matches {
                return Err(ServerError::Forbidden(
                    "invalid swarm message signature (OWASP ASI-05)".into(),
                ));
            }
            wire.signature = provided.to_owned();
        } else {
            wire.signature = computed_sig;
        }

        self.bus.0.publish_swarm_message(&wire).await?;

        let stored = MediatedSwarmMessage {
            message_id: wire.message_id,
            session_id: wire.session_id,
            from_agent_id: wire.from_agent_id,
            target_agent_id: dto.target_agent_id.clone(),
            payload: wire.payload,
            signature: wire.signature,
            mediated_by_user: Some(actor.user_id.clone()),
            emitted_at_ms: wire.emitted_at_ms,
        };
        self.store.0.record_message(stored).await
    }

    /// Lists swarm coordination messages for an owned session.
    pub async fn list_messages(
        &self,
        actor: &AuthenticatedActor,
        session_id: &str,
    ) -> Result<Vec<MediatedSwarmMessage>, ServerError> {
        let _ = self.sessions.get_owned(actor, session_id).await?;
        self.store.0.list_messages(session_id).await
    }

    /// Stores a key/value entry on the session's Valkey shared blackboard.
    pub async fn put_blackboard(
        &self,
        actor: &AuthenticatedActor,
        session_id: &str,
        dto: &PutBlackboardDto,
    ) -> Result<Value, ServerError> {
        let _ = self.sessions.get_owned(actor, session_id).await?;
        let raw = serde_json::to_string(&dto.value)
            .map_err(|e| ServerError::Internal(format!("blackboard serialize failed: {e}")))?;
        let cache_key = format!("blackboard:{session_id}:{}", dto.key);
        self.state
            .0
            .put_idempotency(&cache_key, &raw, Duration::from_secs(86_400))
            .await?;
        Ok(dto.value.clone())
    }

    /// Reads a key/value entry from the session's Valkey shared blackboard.
    pub async fn get_blackboard(
        &self,
        actor: &AuthenticatedActor,
        session_id: &str,
        key: &str,
    ) -> Result<Option<Value>, ServerError> {
        let _ = self.sessions.get_owned(actor, session_id).await?;
        let cache_key = format!("blackboard:{session_id}:{key}");
        let Some(raw) = self.state.0.get_idempotency(&cache_key).await? else {
            return Ok(None);
        };
        let parsed: Value = serde_json::from_str(&raw)
            .map_err(|e| ServerError::Internal(format!("blackboard parse failed: {e}")))?;
        Ok(Some(parsed))
    }

    fn sign_swarm_message(&self, msg: &SwarmMessage) -> String {
        let key = self.config.jwt_secret.as_bytes();
        let topic_label = match &msg.to {
            SwarmTopic::Broadcast => format!("swarm.{}.broadcast", msg.session_id),
            SwarmTopic::DirectAgent(id) => format!("swarm.{}.agent.{id}", msg.session_id),
        };
        let payload_canon = serde_json::to_string(&msg.payload).unwrap_or_default();
        let perms_canon = serde_json::to_string(&msg.permission_mask).unwrap_or_default();
        let canonical = format!(
            "{}|{}|{}|{}|{}|{}",
            msg.session_id,
            msg.from_agent_id,
            topic_label,
            payload_canon,
            perms_canon,
            msg.emitted_at_ms
        );
        hmac_sha256_hex(key, canonical.as_bytes())
    }
}

fn hmac_sha256_hex(key: &[u8], msg: &[u8]) -> String {
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
    let inner_digest = inner.finalize();
    let mut outer = Sha256::new();
    outer.update(o_pad);
    outer.update(inner_digest);
    hex::encode(outer.finalize())
}
