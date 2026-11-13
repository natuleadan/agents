//! Authentication and authorization DTOs (Argon2id + JWT + opaque API keys).

use std::fmt;

use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// Scoped permission granted to an API key or JWT session.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum ApiKeyScope {
    Read,
    Write,
    Exec,
    Admin,
    Unknown(String),
}

impl ApiKeyScope {
    pub fn as_str(&self) -> &str {
        match self {
            Self::Read => "read",
            Self::Write => "write",
            Self::Exec => "exec",
            Self::Admin => "admin",
            Self::Unknown(raw) => raw.as_str(),
        }
    }
}

impl Serialize for ApiKeyScope {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for ApiKeyScope {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let raw = String::deserialize(deserializer)?;
        Ok(match raw.as_str() {
            "read" => Self::Read,
            "write" => Self::Write,
            "exec" => Self::Exec,
            "admin" => Self::Admin,
            _ => Self::Unknown(raw),
        })
    }
}

/// Login credentials payload validated with Argon2id on the server.
#[derive(Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LoginRequest {
    pub email: String,
    pub password: String,
}

impl fmt::Debug for LoginRequest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("LoginRequest")
            .field("email", &self.email)
            .field("password", &"[REDACTED]")
            .finish()
    }
}

/// Token refresh request using a revocable opaque refresh secret.
#[derive(Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RefreshRequest {
    pub refresh_token: String,
}

impl fmt::Debug for RefreshRequest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RefreshRequest")
            .field("refresh_token", &"[REDACTED]")
            .finish()
    }
}

/// Access and refresh token bundle returned to clients after authentication.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TokenPair {
    pub access_token: String,
    pub refresh_token: String,
    pub token_type: String,
    pub expires_in_seconds: u64,
}

impl fmt::Debug for TokenPair {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TokenPair")
            .field("access_token", &"[REDACTED]")
            .field("refresh_token", &"[REDACTED]")
            .field("token_type", &self.token_type)
            .field("expires_in_seconds", &self.expires_in_seconds)
            .finish()
    }
}

/// Verified JWT identity claims used across Ironic controllers and guards.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UserClaims {
    pub subject: String,
    pub email: String,
    pub scopes: Vec<ApiKeyScope>,
    pub issued_at: u64,
    pub expires_at: u64,
}

/// Request to mint a revocable SHA-256-hashed opaque API key.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateApiKeyRequest {
    pub label: String,
    pub scopes: Vec<ApiKeyScope>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires_in_seconds: Option<u64>,
}

/// Non-secret metadata persisted and returned for API key listings.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ApiKeyMetadata {
    pub key_id: String,
    pub label: String,
    pub prefix: String,
    pub scopes: Vec<ApiKeyScope>,
    pub created_at: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub revoked_at: Option<u64>,
}
