//! Persistence repository for swarm handoffs and signed swarm messages.

use std::sync::Arc;

use agents_protocol::PermissionMask;
use sqlx::{PgPool, Row};
use tokio::sync::Mutex;

use crate::{
    infra::{error::ServerError, state::BoxFuture},
    modules::swarm::entities::{MediatedSwarmMessage, SwarmHandoffEntity},
};

/// Concrete DI wrapper around [`SwarmRepository`].
#[derive(Clone)]
pub struct SwarmStore(pub Arc<dyn SwarmRepository>);

impl Default for SwarmStore {
    fn default() -> Self {
        Self(InMemorySwarmRepository::new())
    }
}

/// Storage contract for subagent handoffs and signed swarm coordination messages.
pub trait SwarmRepository: Send + Sync + 'static {
    fn save_handoff<'a>(
        &'a self,
        handoff: SwarmHandoffEntity,
    ) -> BoxFuture<'a, Result<SwarmHandoffEntity, ServerError>>;

    fn list_handoffs<'a>(
        &'a self,
        session_id: &'a str,
    ) -> BoxFuture<'a, Result<Vec<SwarmHandoffEntity>, ServerError>>;

    fn record_message<'a>(
        &'a self,
        msg: MediatedSwarmMessage,
    ) -> BoxFuture<'a, Result<MediatedSwarmMessage, ServerError>>;

    fn list_messages<'a>(
        &'a self,
        session_id: &'a str,
    ) -> BoxFuture<'a, Result<Vec<MediatedSwarmMessage>, ServerError>>;
}

/// PostgreSQL swarm repository.
#[derive(Clone)]
pub struct PgSwarmRepository {
    pool: PgPool,
}

impl PgSwarmRepository {
    #[must_use]
    pub fn new(pool: PgPool) -> Arc<Self> {
        Arc::new(Self { pool })
    }
}

impl SwarmRepository for PgSwarmRepository {
    fn save_handoff<'a>(
        &'a self,
        handoff: SwarmHandoffEntity,
    ) -> BoxFuture<'a, Result<SwarmHandoffEntity, ServerError>> {
        Box::pin(async move {
            let mask_json = serde_json::to_value(&handoff.delegated_mask)
                .map_err(|e| ServerError::Internal(format!("serialize mask failed: {e}")))?;
            sqlx::query(
                "INSERT INTO swarm_handoffs (handoff_id, session_id, owner_id, parent_agent_id, child_agent_id, worktree_branch, delegated_mask, goal, created_at)
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)",
            )
            .bind(&handoff.handoff_id)
            .bind(&handoff.session_id)
            .bind(&handoff.owner_id)
            .bind(&handoff.parent_agent_id)
            .bind(&handoff.child_agent_id)
            .bind(&handoff.worktree_branch)
            .bind(mask_json)
            .bind(&handoff.goal)
            .bind(handoff.created_at as i64)
            .execute(&self.pool)
            .await
            .map_err(|e| ServerError::Internal(format!("save handoff failed: {e}")))?;
            Ok(handoff)
        })
    }

    fn list_handoffs<'a>(
        &'a self,
        session_id: &'a str,
    ) -> BoxFuture<'a, Result<Vec<SwarmHandoffEntity>, ServerError>> {
        Box::pin(async move {
            let rows = sqlx::query(
                "SELECT handoff_id, session_id, owner_id, parent_agent_id, child_agent_id, worktree_branch, delegated_mask, goal, created_at
                 FROM swarm_handoffs WHERE session_id = $1 ORDER BY created_at DESC",
            )
            .bind(session_id)
            .fetch_all(&self.pool)
            .await
            .map_err(|e| ServerError::Internal(format!("list handoffs failed: {e}")))?;

            let mut out = Vec::with_capacity(rows.len());
            for row in rows {
                let mask_val: serde_json::Value = row.get("delegated_mask");
                let delegated_mask: PermissionMask = serde_json::from_value(mask_val)
                    .map_err(|e| ServerError::Internal(format!("invalid mask json: {e}")))?;
                out.push(SwarmHandoffEntity {
                    handoff_id: row.get("handoff_id"),
                    session_id: row.get("session_id"),
                    owner_id: row.get("owner_id"),
                    parent_agent_id: row.get("parent_agent_id"),
                    child_agent_id: row.get("child_agent_id"),
                    worktree_branch: row.get("worktree_branch"),
                    delegated_mask,
                    goal: row.get("goal"),
                    created_at: row.get::<i64, _>("created_at").max(0) as u64,
                });
            }
            Ok(out)
        })
    }

    fn record_message<'a>(
        &'a self,
        msg: MediatedSwarmMessage,
    ) -> BoxFuture<'a, Result<MediatedSwarmMessage, ServerError>> {
        Box::pin(async move {
            let topic = msg
                .target_agent_id
                .clone()
                .unwrap_or_else(|| "broadcast".to_owned());
            sqlx::query(
                "INSERT INTO swarm_messages (message_id, session_id, from_agent_id, topic, payload, permission_mask, signature, mediated_by_user, emitted_at_ms)
                 VALUES ($1, $2, $3, $4, $5, '{}'::jsonb, $6, $7, $8)",
            )
            .bind(&msg.message_id)
            .bind(&msg.session_id)
            .bind(&msg.from_agent_id)
            .bind(topic)
            .bind(&msg.payload)
            .bind(&msg.signature)
            .bind(&msg.mediated_by_user)
            .bind(msg.emitted_at_ms as i64)
            .execute(&self.pool)
            .await
            .map_err(|e| ServerError::Internal(format!("record swarm message failed: {e}")))?;
            Ok(msg)
        })
    }

    fn list_messages<'a>(
        &'a self,
        session_id: &'a str,
    ) -> BoxFuture<'a, Result<Vec<MediatedSwarmMessage>, ServerError>> {
        Box::pin(async move {
            let rows = sqlx::query(
                "SELECT message_id, session_id, from_agent_id, topic, payload, signature, mediated_by_user, emitted_at_ms
                 FROM swarm_messages WHERE session_id = $1 ORDER BY emitted_at_ms ASC",
            )
            .bind(session_id)
            .fetch_all(&self.pool)
            .await
            .map_err(|e| ServerError::Internal(format!("list swarm messages failed: {e}")))?;

            Ok(rows
                .into_iter()
                .map(|row| {
                    let topic: String = row.get("topic");
                    MediatedSwarmMessage {
                        message_id: row.get("message_id"),
                        session_id: row.get("session_id"),
                        from_agent_id: row.get("from_agent_id"),
                        target_agent_id: if topic == "broadcast" {
                            None
                        } else {
                            Some(topic)
                        },
                        payload: row.get("payload"),
                        signature: row.get("signature"),
                        mediated_by_user: row.get("mediated_by_user"),
                        emitted_at_ms: row.get::<i64, _>("emitted_at_ms").max(0) as u64,
                    }
                })
                .collect())
        })
    }
}

/// Deterministic in-memory repository used in standalone dev/test modes.
#[derive(Clone, Default)]
pub struct InMemorySwarmRepository {
    handoffs: Arc<Mutex<Vec<SwarmHandoffEntity>>>,
    messages: Arc<Mutex<Vec<MediatedSwarmMessage>>>,
}

impl InMemorySwarmRepository {
    #[must_use]
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }
}

impl SwarmRepository for InMemorySwarmRepository {
    fn save_handoff<'a>(
        &'a self,
        handoff: SwarmHandoffEntity,
    ) -> BoxFuture<'a, Result<SwarmHandoffEntity, ServerError>> {
        Box::pin(async move {
            self.handoffs.lock().await.push(handoff.clone());
            Ok(handoff)
        })
    }

    fn list_handoffs<'a>(
        &'a self,
        session_id: &'a str,
    ) -> BoxFuture<'a, Result<Vec<SwarmHandoffEntity>, ServerError>> {
        Box::pin(async move {
            Ok(self
                .handoffs
                .lock()
                .await
                .iter()
                .filter(|h| h.session_id == session_id)
                .cloned()
                .collect())
        })
    }

    fn record_message<'a>(
        &'a self,
        msg: MediatedSwarmMessage,
    ) -> BoxFuture<'a, Result<MediatedSwarmMessage, ServerError>> {
        Box::pin(async move {
            self.messages.lock().await.push(msg.clone());
            Ok(msg)
        })
    }

    fn list_messages<'a>(
        &'a self,
        session_id: &'a str,
    ) -> BoxFuture<'a, Result<Vec<MediatedSwarmMessage>, ServerError>> {
        Box::pin(async move {
            Ok(self
                .messages
                .lock()
                .await
                .iter()
                .filter(|m| m.session_id == session_id)
                .cloned()
                .collect())
        })
    }
}
