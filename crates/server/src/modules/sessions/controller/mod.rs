//! Thin Ironic HTTP controller for session management and draft recovery.

use std::sync::Arc;

use agents_protocol::{ApiKeyScope, CursorPage};
use ironic::prelude::*;

use crate::{
    infra::error::validate_dto,
    modules::{
        auth::{
            entities::AuthenticatedActor,
            guards::{AuthGuard, CurrentActor, require_scope},
        },
        sessions::{
            dto::{CreateSessionDto, ListSessionsQuery, SaveDraftDto, UpdateSessionDto},
            entities::{DraftSnapshot, SessionEntity},
            services::SessionService,
        },
    },
};

#[controller("/api/v1/sessions")]
#[guard(AuthGuard)]
#[derive(Injectable)]
pub struct SessionsController {
    service: Arc<SessionService>,
}

#[routes]
impl SessionsController {
    #[post]
    #[api(
        summary = "Create a session",
        tag = "Sessions",
        security = "bearerAuth"
    )]
    #[body(json = CreateSessionDto)]
    #[resp(200, "Created session", json = SessionEntity)]
    async fn create(
        &self,
        #[decorator(CurrentActor)] actor: AuthenticatedActor,
        #[body] dto: CreateSessionDto,
    ) -> Result<Json<SessionEntity>, HttpError> {
        validate_dto(&dto)?;
        require_scope(&actor, ApiKeyScope::Write)?;
        let session = self.service.create(&actor, &dto).await?;
        Ok(Json(session))
    }

    #[get]
    #[api(
        summary = "List caller sessions",
        tag = "Sessions",
        security = "bearerAuth"
    )]
    #[resp(200, "Cursor-paginated sessions", json = CursorPage<SessionEntity>)]
    async fn list(
        &self,
        #[decorator(CurrentActor)] actor: AuthenticatedActor,
        #[query] query: ListSessionsQuery,
    ) -> Result<Json<CursorPage<SessionEntity>>, HttpError> {
        validate_dto(&query)?;
        require_scope(&actor, ApiKeyScope::Read)?;
        let page = self.service.list(&actor, &query).await?;
        Ok(Json(page))
    }

    #[get("/:id")]
    #[api(
        summary = "Get a session by ID",
        tag = "Sessions",
        security = "bearerAuth"
    )]
    #[resp(200, "Session record", json = SessionEntity)]
    #[resp(403, "BOLA ownership violation")]
    async fn get(
        &self,
        #[decorator(CurrentActor)] actor: AuthenticatedActor,
        #[param] id: String,
    ) -> Result<Json<SessionEntity>, HttpError> {
        require_scope(&actor, ApiKeyScope::Read)?;
        let session = self.service.get_owned(&actor, &id).await?;
        Ok(Json(session))
    }

    #[patch("/:id")]
    #[api(
        summary = "Update a session",
        tag = "Sessions",
        security = "bearerAuth"
    )]
    #[body(json = UpdateSessionDto)]
    #[resp(200, "Updated session", json = SessionEntity)]
    #[resp(409, "Optimistic concurrency conflict")]
    async fn update(
        &self,
        #[decorator(CurrentActor)] actor: AuthenticatedActor,
        #[param] id: String,
        #[body] dto: UpdateSessionDto,
    ) -> Result<Json<SessionEntity>, HttpError> {
        validate_dto(&dto)?;
        require_scope(&actor, ApiKeyScope::Write)?;
        let updated = self.service.update(&actor, &id, &dto).await?;
        Ok(Json(updated))
    }

    #[put("/:id/draft")]
    #[api(
        summary = "Persist a session prompt draft",
        tag = "Sessions",
        security = "bearerAuth"
    )]
    #[body(json = SaveDraftDto)]
    #[resp(200, "Saved draft snapshot", json = DraftSnapshot)]
    async fn save_draft(
        &self,
        #[decorator(CurrentActor)] actor: AuthenticatedActor,
        #[param] id: String,
        #[body] dto: SaveDraftDto,
    ) -> Result<Json<DraftSnapshot>, HttpError> {
        validate_dto(&dto)?;
        require_scope(&actor, ApiKeyScope::Write)?;
        let draft = self.service.save_draft(&actor, &id, &dto).await?;
        Ok(Json(draft))
    }

    #[get("/:id/draft")]
    #[api(
        summary = "Recover saved session draft",
        tag = "Sessions",
        security = "bearerAuth"
    )]
    #[resp(200, "Recovered draft snapshot", json = DraftSnapshot)]
    async fn load_draft(
        &self,
        #[decorator(CurrentActor)] actor: AuthenticatedActor,
        #[param] id: String,
    ) -> Result<Json<DraftSnapshot>, HttpError> {
        require_scope(&actor, ApiKeyScope::Read)?;
        let draft = self.service.load_draft(&actor, &id).await?;
        Ok(Json(draft))
    }

    #[delete("/:id")]
    #[api(
        summary = "Delete a session",
        tag = "Sessions",
        security = "bearerAuth"
    )]
    #[resp(204, "Session deleted")]
    async fn delete(
        &self,
        #[decorator(CurrentActor)] actor: AuthenticatedActor,
        #[param] id: String,
    ) -> Result<(), HttpError> {
        require_scope(&actor, ApiKeyScope::Write)?;
        self.service.delete(&actor, &id).await?;
        Ok(())
    }
}
