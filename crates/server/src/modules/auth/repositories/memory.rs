//! In-memory implementation of [`AuthRepository`] for deterministic unit/integration tests.

use std::sync::Arc;

use tokio::sync::Mutex;

use crate::{
    infra::{error::ServerError, state::BoxFuture},
    modules::auth::{
        entities::{ApiKeyRecord, RefreshTokenRecord, UserRecord},
        repositories::AuthRepository,
    },
};

#[derive(Default)]
struct MemoryAuthInner {
    users: Vec<UserRecord>,
    refresh_tokens: Vec<RefreshTokenRecord>,
    api_keys: Vec<ApiKeyRecord>,
}

/// Deterministic in-memory auth repository used in unit/integration tests.
#[derive(Default)]
pub struct InMemoryAuthRepository {
    inner: Mutex<MemoryAuthInner>,
}

impl InMemoryAuthRepository {
    #[must_use]
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }
}

impl AuthRepository for InMemoryAuthRepository {
    fn create_user<'a>(
        &'a self,
        user: UserRecord,
    ) -> BoxFuture<'a, Result<UserRecord, ServerError>> {
        Box::pin(async move {
            let mut inner = self.inner.lock().await;
            if inner
                .users
                .iter()
                .any(|u| u.email.eq_ignore_ascii_case(&user.email))
            {
                return Err(ServerError::Conflict("email is already registered".into()));
            }
            inner.users.push(user.clone());
            Ok(user)
        })
    }

    fn find_user_by_email<'a>(
        &'a self,
        email: &'a str,
    ) -> BoxFuture<'a, Result<Option<UserRecord>, ServerError>> {
        Box::pin(async move {
            let inner = self.inner.lock().await;
            Ok(inner
                .users
                .iter()
                .find(|u| u.email.eq_ignore_ascii_case(email))
                .cloned())
        })
    }

    fn find_user_by_id<'a>(
        &'a self,
        id: &'a str,
    ) -> BoxFuture<'a, Result<Option<UserRecord>, ServerError>> {
        Box::pin(async move {
            let inner = self.inner.lock().await;
            Ok(inner.users.iter().find(|u| u.id == id).cloned())
        })
    }

    fn insert_refresh_token<'a>(
        &'a self,
        token: RefreshTokenRecord,
    ) -> BoxFuture<'a, Result<(), ServerError>> {
        Box::pin(async move {
            self.inner.lock().await.refresh_tokens.push(token);
            Ok(())
        })
    }

    fn find_refresh_by_hash<'a>(
        &'a self,
        token_hash: &'a str,
    ) -> BoxFuture<'a, Result<Option<RefreshTokenRecord>, ServerError>> {
        Box::pin(async move {
            let inner = self.inner.lock().await;
            Ok(inner
                .refresh_tokens
                .iter()
                .find(|t| t.token_hash == token_hash)
                .cloned())
        })
    }

    fn revoke_refresh_token<'a>(
        &'a self,
        id: &'a str,
        revoked_at: u64,
    ) -> BoxFuture<'a, Result<(), ServerError>> {
        Box::pin(async move {
            let mut inner = self.inner.lock().await;
            if let Some(tok) = inner.refresh_tokens.iter_mut().find(|t| t.id == id)
                && tok.revoked_at.is_none()
            {
                tok.revoked_at = Some(revoked_at);
            }
            Ok(())
        })
    }

    fn insert_api_key<'a>(&'a self, key: ApiKeyRecord) -> BoxFuture<'a, Result<(), ServerError>> {
        Box::pin(async move {
            self.inner.lock().await.api_keys.push(key);
            Ok(())
        })
    }

    fn find_api_key_by_hash<'a>(
        &'a self,
        key_hash: &'a str,
    ) -> BoxFuture<'a, Result<Option<ApiKeyRecord>, ServerError>> {
        Box::pin(async move {
            let inner = self.inner.lock().await;
            Ok(inner
                .api_keys
                .iter()
                .find(|k| k.key_hash == key_hash)
                .cloned())
        })
    }

    fn find_api_key_by_id<'a>(
        &'a self,
        id: &'a str,
    ) -> BoxFuture<'a, Result<Option<ApiKeyRecord>, ServerError>> {
        Box::pin(async move {
            let inner = self.inner.lock().await;
            Ok(inner.api_keys.iter().find(|k| k.id == id).cloned())
        })
    }

    fn list_api_keys_for_user<'a>(
        &'a self,
        user_id: &'a str,
    ) -> BoxFuture<'a, Result<Vec<ApiKeyRecord>, ServerError>> {
        Box::pin(async move {
            let inner = self.inner.lock().await;
            let mut keys: Vec<ApiKeyRecord> = inner
                .api_keys
                .iter()
                .filter(|k| k.user_id == user_id)
                .cloned()
                .collect();
            keys.sort_by_key(|k| std::cmp::Reverse(k.created_at));
            Ok(keys)
        })
    }

    fn revoke_api_key<'a>(
        &'a self,
        id: &'a str,
        revoked_at: u64,
    ) -> BoxFuture<'a, Result<(), ServerError>> {
        Box::pin(async move {
            let mut inner = self.inner.lock().await;
            if let Some(key) = inner.api_keys.iter_mut().find(|k| k.id == id)
                && key.revoked_at.is_none()
            {
                key.revoked_at = Some(revoked_at);
            }
            Ok(())
        })
    }
}
