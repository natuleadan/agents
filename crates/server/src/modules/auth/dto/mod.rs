//! Validated request/response DTOs for authentication and API key lifecycle.

use std::fmt;

use agents_protocol::{ApiKeyMetadata, ApiKeyScope};
use garde::Validate;
use ironic::OpenApiSchema;
use serde::{Deserialize, Serialize};

/// Registration request validated with `garde` before Argon2id hashing.
#[derive(Clone, Deserialize, Validate, OpenApiSchema)]
#[serde(rename_all = "camelCase")]
pub struct RegisterDto {
    #[garde(email, length(max = 254))]
    pub email: String,
    #[garde(length(min = 12, max = 128))]
    pub password: String,
}

impl fmt::Debug for RegisterDto {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RegisterDto")
            .field("email", &self.email)
            .field("password", &"[REDACTED]")
            .finish()
    }
}

/// Password login payload validated with `garde`.
#[derive(Clone, Deserialize, Validate, OpenApiSchema)]
#[serde(rename_all = "camelCase")]
pub struct LoginDto {
    #[garde(email, length(max = 254))]
    pub email: String,
    #[garde(length(min = 1, max = 128))]
    pub password: String,
}

impl fmt::Debug for LoginDto {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("LoginDto")
            .field("email", &self.email)
            .field("password", &"[REDACTED]")
            .finish()
    }
}

/// Token refresh / logout payload carrying an opaque refresh secret.
#[derive(Clone, Deserialize, Validate, OpenApiSchema)]
#[serde(rename_all = "camelCase")]
pub struct RefreshDto {
    #[garde(length(min = 16, max = 256))]
    pub refresh_token: String,
}

impl fmt::Debug for RefreshDto {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RefreshDto")
            .field("refresh_token", &"[REDACTED]")
            .finish()
    }
}

/// Request to create a revocable SHA-256-hashed API key.
#[derive(Debug, Clone, Deserialize, Validate, OpenApiSchema)]
#[serde(rename_all = "camelCase")]
pub struct CreateApiKeyDto {
    #[garde(length(min = 2, max = 80))]
    pub label: String,
    #[garde(length(min = 1, max = 8))]
    pub scopes: Vec<ApiKeyScope>,
    #[garde(range(min = 60, max = 31_536_000))]
    pub expires_in_seconds: Option<u64>,
}

/// One-time response exposing the newly generated plaintext API key alongside its metadata.
#[derive(Clone, Serialize, Deserialize, OpenApiSchema)]
#[serde(rename_all = "camelCase")]
pub struct CreatedApiKeyResponse {
    pub metadata: ApiKeyMetadata,
    pub plaintext_key: String,
}

impl fmt::Debug for CreatedApiKeyResponse {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CreatedApiKeyResponse")
            .field("metadata", &self.metadata)
            .field("plaintext_key", &"[REDACTED]")
            .finish()
    }
}

/// Confirmation returned when revoking an API key.
#[derive(Debug, Clone, Serialize, Deserialize, OpenApiSchema)]
#[serde(rename_all = "camelCase")]
pub struct RevokeApiKeyResponse {
    pub key_id: String,
    pub revoked_at: u64,
}
