//! PostgreSQL repository implementing [`AuthRepository`].

use std::sync::Arc;

use agents_protocol::ApiKeyScope;
use sqlx::{PgPool, Row};

use crate::{
    infra::{error::ServerError, state::BoxFuture},
    modules::auth::{
        entities::{ApiKeyRecord, RefreshTokenRecord, UserRecord},
        repositories::AuthRepository,
    },
};

/// PostgreSQL repository implementing [`AuthRepository`].
#[derive(Clone)]
pub struct PgAuthRepository {
    pool: PgPool,
}

impl PgAuthRepository {
    #[must_use]
    pub fn new(pool: PgPool) -> Arc<Self> {
        Arc::new(Self { pool })
    }
}

impl AuthRepository for PgAuthRepository {
    fn create_user<'a>(
        &'a self,
        user: UserRecord,
    ) -> BoxFuture<'a, Result<UserRecord, ServerError>> {
        Box::pin(async move {
            let scopes_json = serde_json::to_value(&user.scopes)
                .map_err(|e| ServerError::Internal(e.to_string()))?;
            sqlx::query(
                "INSERT INTO users (id, email, password_hash, role, scopes, created_at)
                 VALUES ($1, $2, $3, $4, $5, $6)",
            )
            .bind(&user.id)
            .bind(&user.email)
            .bind(&user.password_hash)
            .bind(&user.role)
            .bind(scopes_json)
            .bind(user.created_at as i64)
            .execute(&self.pool)
            .await
            .map_err(|e| {
                let msg = e.to_string();
                if msg.contains("duplicate key") || msg.contains("unique") {
                    ServerError::Conflict("email is already registered".into())
                } else {
                    ServerError::Internal(format!("create_user failed: {msg}"))
                }
            })?;
            Ok(user)
        })
    }

    fn find_user_by_email<'a>(
        &'a self,
        email: &'a str,
    ) -> BoxFuture<'a, Result<Option<UserRecord>, ServerError>> {
        Box::pin(async move {
            let row = sqlx::query(
                "SELECT id, email, password_hash, role, scopes, created_at FROM users WHERE email = $1",
            )
            .bind(email)
            .fetch_optional(&self.pool)
            .await
            .map_err(|e| ServerError::Internal(format!("find_user_by_email failed: {e}")))?;

            row.map(row_to_user).transpose()
        })
    }

    fn find_user_by_id<'a>(
        &'a self,
        id: &'a str,
    ) -> BoxFuture<'a, Result<Option<UserRecord>, ServerError>> {
        Box::pin(async move {
            let row = sqlx::query(
                "SELECT id, email, password_hash, role, scopes, created_at FROM users WHERE id = $1",
            )
            .bind(id)
            .fetch_optional(&self.pool)
            .await
            .map_err(|e| ServerError::Internal(format!("find_user_by_id failed: {e}")))?;

            row.map(row_to_user).transpose()
        })
    }

    fn insert_refresh_token<'a>(
        &'a self,
        token: RefreshTokenRecord,
    ) -> BoxFuture<'a, Result<(), ServerError>> {
        Box::pin(async move {
            let scopes_json = serde_json::to_value(&token.scopes)
                .map_err(|e| ServerError::Internal(e.to_string()))?;
            sqlx::query(
                "INSERT INTO refresh_tokens (id, user_id, token_hash, scopes, created_at, expires_at, revoked_at)
                 VALUES ($1, $2, $3, $4, $5, $6, $7)",
            )
            .bind(&token.id)
            .bind(&token.user_id)
            .bind(&token.token_hash)
            .bind(scopes_json)
            .bind(token.created_at as i64)
            .bind(token.expires_at as i64)
            .bind(token.revoked_at.map(|v| v as i64))
            .execute(&self.pool)
            .await
            .map_err(|e| ServerError::Internal(format!("insert_refresh_token failed: {e}")))?;
            Ok(())
        })
    }

    fn find_refresh_by_hash<'a>(
        &'a self,
        token_hash: &'a str,
    ) -> BoxFuture<'a, Result<Option<RefreshTokenRecord>, ServerError>> {
        Box::pin(async move {
            let row = sqlx::query(
                "SELECT id, user_id, token_hash, scopes, created_at, expires_at, revoked_at
                 FROM refresh_tokens WHERE token_hash = $1",
            )
            .bind(token_hash)
            .fetch_optional(&self.pool)
            .await
            .map_err(|e| ServerError::Internal(format!("find_refresh_by_hash failed: {e}")))?;

            row.map(row_to_refresh).transpose()
        })
    }

    fn revoke_refresh_token<'a>(
        &'a self,
        id: &'a str,
        revoked_at: u64,
    ) -> BoxFuture<'a, Result<(), ServerError>> {
        Box::pin(async move {
            sqlx::query(
                "UPDATE refresh_tokens SET revoked_at = $2 WHERE id = $1 AND revoked_at IS NULL",
            )
            .bind(id)
            .bind(revoked_at as i64)
            .execute(&self.pool)
            .await
            .map_err(|e| ServerError::Internal(format!("revoke_refresh_token failed: {e}")))?;
            Ok(())
        })
    }

    fn insert_api_key<'a>(&'a self, key: ApiKeyRecord) -> BoxFuture<'a, Result<(), ServerError>> {
        Box::pin(async move {
            let scopes_json = serde_json::to_value(&key.scopes)
                .map_err(|e| ServerError::Internal(e.to_string()))?;
            sqlx::query(
                "INSERT INTO api_keys (id, user_id, label, prefix, key_hash, scopes, created_at, expires_at, revoked_at)
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)",
            )
            .bind(&key.id)
            .bind(&key.user_id)
            .bind(&key.label)
            .bind(&key.prefix)
            .bind(&key.key_hash)
            .bind(scopes_json)
            .bind(key.created_at as i64)
            .bind(key.expires_at.map(|v| v as i64))
            .bind(key.revoked_at.map(|v| v as i64))
            .execute(&self.pool)
            .await
            .map_err(|e| ServerError::Internal(format!("insert_api_key failed: {e}")))?;
            Ok(())
        })
    }

    fn find_api_key_by_hash<'a>(
        &'a self,
        key_hash: &'a str,
    ) -> BoxFuture<'a, Result<Option<ApiKeyRecord>, ServerError>> {
        Box::pin(async move {
            let row = sqlx::query(
                "SELECT id, user_id, label, prefix, key_hash, scopes, created_at, expires_at, revoked_at
                 FROM api_keys WHERE key_hash = $1",
            )
            .bind(key_hash)
            .fetch_optional(&self.pool)
            .await
            .map_err(|e| ServerError::Internal(format!("find_api_key_by_hash failed: {e}")))?;

            row.map(row_to_api_key).transpose()
        })
    }

    fn find_api_key_by_id<'a>(
        &'a self,
        id: &'a str,
    ) -> BoxFuture<'a, Result<Option<ApiKeyRecord>, ServerError>> {
        Box::pin(async move {
            let row = sqlx::query(
                "SELECT id, user_id, label, prefix, key_hash, scopes, created_at, expires_at, revoked_at
                 FROM api_keys WHERE id = $1",
            )
            .bind(id)
            .fetch_optional(&self.pool)
            .await
            .map_err(|e| ServerError::Internal(format!("find_api_key_by_id failed: {e}")))?;

            row.map(row_to_api_key).transpose()
        })
    }

    fn list_api_keys_for_user<'a>(
        &'a self,
        user_id: &'a str,
    ) -> BoxFuture<'a, Result<Vec<ApiKeyRecord>, ServerError>> {
        Box::pin(async move {
            let rows = sqlx::query(
                "SELECT id, user_id, label, prefix, key_hash, scopes, created_at, expires_at, revoked_at
                 FROM api_keys WHERE user_id = $1 ORDER BY created_at DESC",
            )
            .bind(user_id)
            .fetch_all(&self.pool)
            .await
            .map_err(|e| ServerError::Internal(format!("list_api_keys_for_user failed: {e}")))?;

            rows.into_iter().map(row_to_api_key).collect()
        })
    }

    fn revoke_api_key<'a>(
        &'a self,
        id: &'a str,
        revoked_at: u64,
    ) -> BoxFuture<'a, Result<(), ServerError>> {
        Box::pin(async move {
            sqlx::query("UPDATE api_keys SET revoked_at = $2 WHERE id = $1 AND revoked_at IS NULL")
                .bind(id)
                .bind(revoked_at as i64)
                .execute(&self.pool)
                .await
                .map_err(|e| ServerError::Internal(format!("revoke_api_key failed: {e}")))?;
            Ok(())
        })
    }
}

fn row_to_user(row: sqlx::postgres::PgRow) -> Result<UserRecord, ServerError> {
    let scopes_val: serde_json::Value = row
        .try_get("scopes")
        .map_err(|e| ServerError::Internal(e.to_string()))?;
    let scopes: Vec<ApiKeyScope> =
        serde_json::from_value(scopes_val).map_err(|e| ServerError::Internal(e.to_string()))?;
    let created: i64 = row
        .try_get("created_at")
        .map_err(|e| ServerError::Internal(e.to_string()))?;
    Ok(UserRecord {
        id: row
            .try_get("id")
            .map_err(|e| ServerError::Internal(e.to_string()))?,
        email: row
            .try_get("email")
            .map_err(|e| ServerError::Internal(e.to_string()))?,
        password_hash: row
            .try_get("password_hash")
            .map_err(|e| ServerError::Internal(e.to_string()))?,
        role: row
            .try_get("role")
            .map_err(|e| ServerError::Internal(e.to_string()))?,
        scopes,
        created_at: created.max(0) as u64,
    })
}

fn row_to_refresh(row: sqlx::postgres::PgRow) -> Result<RefreshTokenRecord, ServerError> {
    let scopes_val: serde_json::Value = row
        .try_get("scopes")
        .map_err(|e| ServerError::Internal(e.to_string()))?;
    let scopes: Vec<ApiKeyScope> =
        serde_json::from_value(scopes_val).map_err(|e| ServerError::Internal(e.to_string()))?;
    let created: i64 = row
        .try_get("created_at")
        .map_err(|e| ServerError::Internal(e.to_string()))?;
    let expires: i64 = row
        .try_get("expires_at")
        .map_err(|e| ServerError::Internal(e.to_string()))?;
    let revoked: Option<i64> = row
        .try_get("revoked_at")
        .map_err(|e| ServerError::Internal(e.to_string()))?;
    Ok(RefreshTokenRecord {
        id: row
            .try_get("id")
            .map_err(|e| ServerError::Internal(e.to_string()))?,
        user_id: row
            .try_get("user_id")
            .map_err(|e| ServerError::Internal(e.to_string()))?,
        token_hash: row
            .try_get("token_hash")
            .map_err(|e| ServerError::Internal(e.to_string()))?,
        scopes,
        created_at: created.max(0) as u64,
        expires_at: expires.max(0) as u64,
        revoked_at: revoked.map(|v| v.max(0) as u64),
    })
}

fn row_to_api_key(row: sqlx::postgres::PgRow) -> Result<ApiKeyRecord, ServerError> {
    let scopes_val: serde_json::Value = row
        .try_get("scopes")
        .map_err(|e| ServerError::Internal(e.to_string()))?;
    let scopes: Vec<ApiKeyScope> =
        serde_json::from_value(scopes_val).map_err(|e| ServerError::Internal(e.to_string()))?;
    let created: i64 = row
        .try_get("created_at")
        .map_err(|e| ServerError::Internal(e.to_string()))?;
    let expires: Option<i64> = row
        .try_get("expires_at")
        .map_err(|e| ServerError::Internal(e.to_string()))?;
    let revoked: Option<i64> = row
        .try_get("revoked_at")
        .map_err(|e| ServerError::Internal(e.to_string()))?;
    Ok(ApiKeyRecord {
        id: row
            .try_get("id")
            .map_err(|e| ServerError::Internal(e.to_string()))?,
        user_id: row
            .try_get("user_id")
            .map_err(|e| ServerError::Internal(e.to_string()))?,
        label: row
            .try_get("label")
            .map_err(|e| ServerError::Internal(e.to_string()))?,
        prefix: row
            .try_get("prefix")
            .map_err(|e| ServerError::Internal(e.to_string()))?,
        key_hash: row
            .try_get("key_hash")
            .map_err(|e| ServerError::Internal(e.to_string()))?,
        scopes,
        created_at: created.max(0) as u64,
        expires_at: expires.map(|v| v.max(0) as u64),
        revoked_at: revoked.map(|v| v.max(0) as u64),
    })
}
