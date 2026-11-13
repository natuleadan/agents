//! Thin Ironic HTTP controller for authentication and API key management.

use std::sync::Arc;

use agents_protocol::{ApiKeyMetadata, ApiKeyScope, TokenPair};
use ironic::prelude::*;

use crate::{
    infra::error::validate_dto,
    modules::auth::{
        dto::{
            CreateApiKeyDto, CreatedApiKeyResponse, LoginDto, RefreshDto, RegisterDto,
            RevokeApiKeyResponse,
        },
        entities::AuthenticatedActor,
        guards::{AuthGuard, CurrentActor, require_scope},
        services::AuthService,
    },
};

#[controller("/api/v1/auth")]
#[derive(Injectable)]
pub struct AuthController {
    service: Arc<AuthService>,
}

#[routes]
impl AuthController {
    #[post("/register")]
    #[api(summary = "Register a user account", tag = "Auth")]
    #[body(json = RegisterDto)]
    #[resp(200, "Registered actor", json = AuthenticatedActor)]
    #[resp(409, "Duplicate email")]
    #[resp(422, "Validation error")]
    async fn register(
        &self,
        #[body] dto: RegisterDto,
    ) -> Result<Json<AuthenticatedActor>, HttpError> {
        validate_dto(&dto)?;
        let actor = self.service.register(&dto).await?;
        Ok(Json(actor))
    }

    #[post("/login")]
    #[api(summary = "Authenticate with email and password", tag = "Auth")]
    #[body(json = LoginDto)]
    #[resp(200, "Access and refresh token pair", json = TokenPair)]
    #[resp(401, "Invalid credentials")]
    async fn login(&self, #[body] dto: LoginDto) -> Result<Json<TokenPair>, HttpError> {
        validate_dto(&dto)?;
        let pair = self.service.login(&dto).await?;
        Ok(Json(pair))
    }

    #[post("/refresh")]
    #[api(summary = "Rotate an opaque refresh token", tag = "Auth")]
    #[body(json = RefreshDto)]
    #[resp(200, "Fresh access and refresh token pair", json = TokenPair)]
    #[resp(401, "Invalid or revoked refresh token")]
    async fn refresh(&self, #[body] dto: RefreshDto) -> Result<Json<TokenPair>, HttpError> {
        validate_dto(&dto)?;
        let pair = self.service.refresh(&dto).await?;
        Ok(Json(pair))
    }

    #[post("/logout")]
    #[api(summary = "Revoke an opaque refresh token", tag = "Auth")]
    #[body(json = RefreshDto)]
    #[resp(204, "Refresh token revoked")]
    async fn logout(&self, #[body] dto: RefreshDto) -> Result<(), HttpError> {
        validate_dto(&dto)?;
        self.service.logout(&dto).await?;
        Ok(())
    }

    #[get("/me")]
    #[guard(AuthGuard)]
    #[api(
        summary = "Inspect authenticated caller identity",
        tag = "Auth",
        security = "bearerAuth"
    )]
    #[resp(200, "Current actor", json = AuthenticatedActor)]
    async fn me(
        &self,
        #[decorator(CurrentActor)] actor: AuthenticatedActor,
    ) -> Result<Json<AuthenticatedActor>, HttpError> {
        Ok(Json(actor))
    }

    #[post("/api-keys")]
    #[guard(AuthGuard)]
    #[api(
        summary = "Mint a revocable SHA-256 API key",
        tag = "Auth",
        security = "bearerAuth"
    )]
    #[body(json = CreateApiKeyDto)]
    #[resp(200, "Created API key", json = CreatedApiKeyResponse)]
    async fn create_api_key(
        &self,
        #[decorator(CurrentActor)] actor: AuthenticatedActor,
        #[body] dto: CreateApiKeyDto,
    ) -> Result<Json<CreatedApiKeyResponse>, HttpError> {
        validate_dto(&dto)?;
        require_scope(&actor, ApiKeyScope::Write)?;
        let created = self.service.create_api_key(&actor, &dto).await?;
        Ok(Json(created))
    }

    #[get("/api-keys")]
    #[guard(AuthGuard)]
    #[api(
        summary = "List caller API key metadata",
        tag = "Auth",
        security = "bearerAuth"
    )]
    #[resp(200, "API key metadata list", json = Vec<ApiKeyMetadata>)]
    async fn list_api_keys(
        &self,
        #[decorator(CurrentActor)] actor: AuthenticatedActor,
    ) -> Result<Json<Vec<ApiKeyMetadata>>, HttpError> {
        require_scope(&actor, ApiKeyScope::Read)?;
        let keys = self.service.list_api_keys(&actor).await?;
        Ok(Json(keys))
    }

    #[delete("/api-keys/:id")]
    #[guard(AuthGuard)]
    #[api(
        summary = "Revoke an API key by ID",
        tag = "Auth",
        security = "bearerAuth"
    )]
    #[resp(200, "Revocation summary", json = RevokeApiKeyResponse)]
    async fn revoke_api_key(
        &self,
        #[decorator(CurrentActor)] actor: AuthenticatedActor,
        #[param] id: String,
    ) -> Result<Json<RevokeApiKeyResponse>, HttpError> {
        require_scope(&actor, ApiKeyScope::Write)?;
        let res = self.service.revoke_api_key(&actor, &id).await?;
        Ok(Json(res))
    }
}
