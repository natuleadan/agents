//! Authentication domain entities and authenticated actor extension.

use agents_protocol::ApiKeyScope;
use ironic::OpenApiSchema;
use serde::{Deserialize, Serialize};

/// Persisted user account authenticated via Argon2id password hash.
#[derive(Clone, PartialEq, Eq)]
pub struct UserRecord {
    pub id: String,
    pub email: String,
    pub password_hash: String,
    pub role: String,
    pub scopes: Vec<ApiKeyScope>,
    pub created_at: u64,
}

impl std::fmt::Debug for UserRecord {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("UserRecord")
            .field("id", &self.id)
            .field("email", &self.email)
            .field("password_hash", &"[REDACTED]")
            .field("role", &self.role)
            .field("scopes", &self.scopes)
            .field("created_at", &self.created_at)
            .finish()
    }
}

/// Persisted SHA-256 hashed revocable refresh token.
#[derive(Clone, PartialEq, Eq)]
pub struct RefreshTokenRecord {
    pub id: String,
    pub user_id: String,
    pub token_hash: String,
    pub scopes: Vec<ApiKeyScope>,
    pub created_at: u64,
    pub expires_at: u64,
    pub revoked_at: Option<u64>,
}

/// Persisted SHA-256 hashed programmatic API key.
#[derive(Clone, PartialEq, Eq)]
pub struct ApiKeyRecord {
    pub id: String,
    pub user_id: String,
    pub label: String,
    pub prefix: String,
    pub key_hash: String,
    pub scopes: Vec<ApiKeyScope>,
    pub created_at: u64,
    pub expires_at: Option<u64>,
    pub revoked_at: Option<u64>,
}

/// Verified caller identity inserted into Ironic `RequestContext` by [`crate::modules::auth::guards::AuthGuard`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, OpenApiSchema)]
#[serde(rename_all = "camelCase")]
pub struct AuthenticatedActor {
    pub user_id: String,
    pub email: String,
    pub scopes: Vec<ApiKeyScope>,
    pub auth_method: String,
}

impl AuthenticatedActor {
    #[must_use]
    pub fn has_scope(&self, required: &ApiKeyScope) -> bool {
        self.scopes
            .iter()
            .any(|scope| scope == &ApiKeyScope::Admin || scope == required)
    }
}
