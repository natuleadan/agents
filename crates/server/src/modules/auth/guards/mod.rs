//! Ironic AuthGuard, parameter extractors, and BOLA object-ownership helpers.

use agents_protocol::ApiKeyScope;
use ironic::{
    ExtractFuture, ExtractedValue, Guard, GuardDecision, GuardFuture, HttpError,
    ParameterExtractor, RequestContext, RequestScope,
};

use crate::modules::auth::{entities::AuthenticatedActor, services::AuthService};

/// Route guard authenticating `Authorization: Bearer ...` or `X-Api-Key: ...` credentials.
#[derive(Default)]
pub struct AuthGuard;

impl Guard for AuthGuard {
    fn can_activate<'a>(&'a self, context: &'a mut RequestContext) -> GuardFuture<'a> {
        Box::pin(async move {
            let token = extract_credential(context.request().headers())?;
            let scope = context
                .extension::<RequestScope>()
                .cloned()
                .ok_or_else(|| {
                    HttpError::internal(
                        "AUTH_SCOPE_MISSING",
                        "Request dependency scope unavailable",
                    )
                })?;
            let auth_service = scope.resolve::<AuthService>().await.map_err(|_| {
                HttpError::internal("AUTH_SERVICE_UNAVAILABLE", "AuthService unavailable")
            })?;
            let actor = auth_service
                .authenticate_token(&token)
                .await
                .map_err(HttpError::from)?;
            context.insert_extension(actor);
            Ok(GuardDecision::Allow)
        })
    }
}

/// Extracts `Authorization: Bearer <token>` or `X-Api-Key: <key>` from HTTP headers.
///
/// # Errors
///
/// Returns `401 Unauthorized` when missing or malformed.
pub fn extract_credential(headers: &ironic::HeaderMap) -> Result<String, HttpError> {
    if let Some(auth_val) = headers.get("authorization").and_then(|v| v.to_str().ok()) {
        if let Some(bearer) = auth_val
            .strip_prefix("Bearer ")
            .or_else(|| auth_val.strip_prefix("bearer "))
        {
            let trimmed = bearer.trim();
            if !trimmed.is_empty() {
                return Ok(trimmed.to_owned());
            }
        }
        return Err(HttpError::unauthorized(
            "AUTH_INVALID_HEADER",
            "Authorization header must use Bearer scheme",
        ));
    }
    if let Some(api_key) = headers.get("x-api-key").and_then(|v| v.to_str().ok()) {
        let trimmed = api_key.trim();
        if !trimmed.is_empty() {
            return Ok(trimmed.to_owned());
        }
    }
    Err(HttpError::unauthorized(
        "AUTH_MISSING_CREDENTIALS",
        "Authentication credentials required",
    ))
}

/// Custom `#[decorator(CurrentActor)]` extractor returning the [`AuthenticatedActor`].
#[derive(Default)]
pub struct CurrentActor;

impl CurrentActor {
    #[must_use]
    pub const fn new() -> Self {
        Self
    }
}

impl ParameterExtractor for CurrentActor {
    fn extract<'a>(&'a self, context: &'a mut RequestContext) -> ExtractFuture<'a> {
        Box::pin(async move {
            let actor = current_actor(context)?;
            Ok(Box::new(actor) as ExtractedValue)
        })
    }

    fn description(&self) -> &'static str {
        "authenticated actor"
    }
}

/// Custom `#[decorator(OptionalIdempotencyKey)]` extractor reading `Idempotency-Key` header if set.
#[derive(Default)]
pub struct OptionalIdempotencyKey;

impl OptionalIdempotencyKey {
    #[must_use]
    pub const fn new() -> Self {
        Self
    }
}

impl ParameterExtractor for OptionalIdempotencyKey {
    fn extract<'a>(&'a self, context: &'a mut RequestContext) -> ExtractFuture<'a> {
        Box::pin(async move {
            let key = context
                .request()
                .headers()
                .get("idempotency-key")
                .and_then(|v| v.to_str().ok())
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(ToOwned::to_owned);
            Ok(Box::new(key) as ExtractedValue)
        })
    }

    fn description(&self) -> &'static str {
        "optional idempotency key header"
    }
}

/// Retrieves the verified [`AuthenticatedActor`] from an Ironic [`RequestContext`].
///
/// # Errors
///
/// Returns `401 Unauthorized` when missing.
pub fn current_actor(context: &RequestContext) -> Result<AuthenticatedActor, HttpError> {
    context
        .extension::<AuthenticatedActor>()
        .cloned()
        .ok_or_else(|| HttpError::unauthorized("AUTH_UNAUTHORIZED", "Actor not authenticated"))
}

/// Enforces RBAC scope on an authenticated caller (`403 Forbidden` when missing).
///
/// # Errors
///
/// Returns `403 Forbidden` if `actor` lacks `required`.
pub fn require_scope(actor: &AuthenticatedActor, required: ApiKeyScope) -> Result<(), HttpError> {
    if actor.has_scope(&required) {
        Ok(())
    } else {
        Err(HttpError::forbidden(
            "AUTH_INSUFFICIENT_SCOPE",
            format!("Required scope `{}` missing", required.as_str()),
        ))
    }
}

/// Strict BOLA object-ownership check (`403 Forbidden` on cross-user access).
///
/// # Errors
///
/// Returns `403 Forbidden` when `actor.user_id != owner_id`.
pub fn ensure_owner(actor: &AuthenticatedActor, owner_id: &str) -> Result<(), HttpError> {
    if actor.user_id == owner_id {
        Ok(())
    } else {
        Err(HttpError::forbidden(
            "AUTH_BOLA_FORBIDDEN",
            "Access denied: caller does not own target resource",
        ))
    }
}
