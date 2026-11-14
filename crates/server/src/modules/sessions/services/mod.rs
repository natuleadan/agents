//! Session service enforcing ownership (BOLA prevention), optimistic concurrency, and drafts.

use std::{sync::Arc, time::Duration};

use agents_protocol::{CursorPage, InteractionMode};
use ironic::prelude::*;
use serde_json::json;

use crate::{
    infra::{
        cache::AppCache,
        error::ServerError,
        events::{AppEventBus, SessionCreatedEvent, SessionUpdatedEvent},
        state::now_ms,
    },
    modules::{
        audit::services::AuditService,
        auth::{entities::AuthenticatedActor, guards::ensure_owner, services::StateStore},
        sessions::{
            dto::{CreateSessionDto, ListSessionsQuery, SaveDraftDto, UpdateSessionDto},
            entities::{DraftSnapshot, SessionEntity},
            repositories::SessionStore,
        },
    },
};

/// Domain business logic for sessions and local draft recovery.
#[derive(Injectable)]
pub struct SessionService {
    store: Arc<SessionStore>,
    state: Arc<StateStore>,
    audit: Arc<AuditService>,
    cache: Arc<AppCache>,
    events: Arc<AppEventBus>,
}

impl SessionService {
    /// Creates a new session owned by `actor`.
    ///
    /// # Errors
    ///
    /// Returns [`ServerError`] on repository failure.
    pub async fn create(
        &self,
        actor: &AuthenticatedActor,
        dto: &CreateSessionDto,
    ) -> Result<SessionEntity, ServerError> {
        let now = now_ms();
        let session = SessionEntity {
            id: uuid::Uuid::new_v4().to_string(),
            owner_id: actor.user_id.clone(),
            title: dto.title.trim().to_owned(),
            mode: dto.mode.unwrap_or(InteractionMode::Act),
            archived_at: None,
            draft_content: None,
            draft_updated_at: None,
            version: 1,
            created_at: now,
            updated_at: now,
        };
        let created = self.store.0.create(session).await?;
        self.cache
            .set_json(
                &format!("session:{}", created.id),
                &created,
                Duration::from_secs(60),
            )
            .await;
        let _ = self
            .events
            .publish(SessionCreatedEvent {
                session_id: created.id.clone(),
                user_id: actor.user_id.clone(),
                mode: format!("{:?}", created.mode),
                created_at: now,
            })
            .await;
        let _ = self
            .audit
            .record(
                Some(&actor.user_id),
                "session.create",
                "session",
                Some(&created.id),
                "success",
                json!({}),
            )
            .await;
        Ok(created)
    }

    /// Lists sessions belonging strictly to `actor` with cursor pagination and tri-state filters.
    ///
    /// # Errors
    ///
    /// Returns [`ServerError::Validation`] on invalid filter or [`ServerError::Internal`] on DB error.
    pub async fn list(
        &self,
        actor: &AuthenticatedActor,
        query: &ListSessionsQuery,
    ) -> Result<CursorPage<SessionEntity>, ServerError> {
        let archived = query.archived_filter()?;
        let limit = query.limit.unwrap_or(25);
        self.store
            .0
            .list_for_owner(&actor.user_id, archived, query.cursor.as_deref(), limit)
            .await
    }

    /// Loads a session after strict BOLA ownership verification.
    ///
    /// # Errors
    ///
    /// Returns [`ServerError::NotFound`] or [`ServerError::Forbidden`].
    pub async fn get_owned(
        &self,
        actor: &AuthenticatedActor,
        session_id: &str,
    ) -> Result<SessionEntity, ServerError> {
        let cache_key = format!("session:{session_id}");
        if let Some(cached) = self.cache.get_json::<SessionEntity>(&cache_key).await {
            if ensure_owner(actor, &cached.owner_id).is_err() {
                return Err(ServerError::Forbidden(
                    "access denied: caller does not own this session".into(),
                ));
            }
            return Ok(cached);
        }

        let session =
            self.store.0.find_by_id(session_id).await?.ok_or_else(|| {
                ServerError::NotFound(format!("session `{session_id}` not found"))
            })?;

        if ensure_owner(actor, &session.owner_id).is_err() {
            let _ = self
                .audit
                .record(
                    Some(&actor.user_id),
                    "session.access",
                    "session",
                    Some(session_id),
                    "denied_bola",
                    json!({ "ownerId": session.owner_id }),
                )
                .await;
            return Err(ServerError::Forbidden(
                "access denied: caller does not own this session".into(),
            ));
        }
        self.cache
            .set_json(&cache_key, &session, Duration::from_secs(60))
            .await;
        Ok(session)
    }

    /// Updates session metadata using optimistic concurrency (`expected_version`).
    ///
    /// # Errors
    ///
    /// Returns [`ServerError::Forbidden`], [`ServerError::NotFound`], or [`ServerError::Conflict`].
    pub async fn update(
        &self,
        actor: &AuthenticatedActor,
        session_id: &str,
        dto: &UpdateSessionDto,
    ) -> Result<SessionEntity, ServerError> {
        let mut session = self.get_owned(actor, session_id).await?;
        let now = now_ms();
        if let Some(title) = &dto.title {
            session.title = title.trim().to_owned();
        }
        if let Some(mode) = dto.mode {
            session.mode = mode;
        }
        if let Some(archived) = dto.archived {
            session.archived_at = if archived { Some(now) } else { None };
        }
        session.version = session.version.saturating_add(1);
        session.updated_at = now;
        let updated = self.store.0.update(session, dto.expected_version).await?;
        self.cache.remove(&format!("session:{session_id}")).await;
        let _ = self
            .events
            .publish(SessionUpdatedEvent {
                session_id: updated.id.clone(),
                user_id: actor.user_id.clone(),
                status: "active".into(),
                updated_at: now,
            })
            .await;
        Ok(updated)
    }

    /// Saves an unsent prompt draft in both Valkey (fast cache) and PostgreSQL (durable).
    ///
    /// # Errors
    ///
    /// Returns [`ServerError::Forbidden`] or [`ServerError::NotFound`].
    pub async fn save_draft(
        &self,
        actor: &AuthenticatedActor,
        session_id: &str,
        dto: &SaveDraftDto,
    ) -> Result<DraftSnapshot, ServerError> {
        let _ = self.get_owned(actor, session_id).await?;
        let now = now_ms();
        self.state.0.save_draft(session_id, &dto.content).await?;
        self.store
            .0
            .save_draft(session_id, &dto.content, now)
            .await?;
        Ok(DraftSnapshot {
            session_id: session_id.to_owned(),
            content: Some(dto.content.clone()),
            updated_at: Some(now),
        })
    }

    /// Recovers the latest draft from Valkey or durable database fallback.
    ///
    /// # Errors
    ///
    /// Returns [`ServerError::Forbidden`] or [`ServerError::NotFound`].
    pub async fn load_draft(
        &self,
        actor: &AuthenticatedActor,
        session_id: &str,
    ) -> Result<DraftSnapshot, ServerError> {
        let session = self.get_owned(actor, session_id).await?;
        let cached = self.state.0.load_draft(session_id).await?;
        Ok(DraftSnapshot {
            session_id: session.id,
            content: cached.or(session.draft_content),
            updated_at: session.draft_updated_at,
        })
    }

    /// Deletes a session after verifying caller ownership.
    ///
    /// # Errors
    ///
    /// Returns [`ServerError::Forbidden`] or [`ServerError::NotFound`].
    pub async fn delete(
        &self,
        actor: &AuthenticatedActor,
        session_id: &str,
    ) -> Result<(), ServerError> {
        let _ = self.get_owned(actor, session_id).await?;
        let removed = self.store.0.delete(session_id).await?;
        if !removed {
            return Err(ServerError::NotFound(format!(
                "session `{session_id}` not found"
            )));
        }
        self.cache.remove(&format!("session:{session_id}")).await;
        let _ = self
            .events
            .publish(SessionUpdatedEvent {
                session_id: session_id.to_owned(),
                user_id: actor.user_id.clone(),
                status: "deleted".into(),
                updated_at: now_ms(),
            })
            .await;
        let _ = self
            .audit
            .record(
                Some(&actor.user_id),
                "session.delete",
                "session",
                Some(session_id),
                "success",
                json!({}),
            )
            .await;
        Ok(())
    }
}
