//! Authentication persistence repository (PostgreSQL + in-memory test repository).

pub mod memory;
pub mod pg;

use std::sync::Arc;

use crate::{
    infra::{error::ServerError, state::BoxFuture},
    modules::auth::entities::{ApiKeyRecord, RefreshTokenRecord, UserRecord},
};

pub use memory::InMemoryAuthRepository;
pub use pg::PgAuthRepository;

/// Concrete DI wrapper around [`AuthRepository`].
#[derive(Clone)]
pub struct AuthStore(pub Arc<dyn AuthRepository>);

impl Default for AuthStore {
    fn default() -> Self {
        Self(InMemoryAuthRepository::new())
    }
}

/// Persistence contract for users, hashed refresh tokens, and hashed API keys.
pub trait AuthRepository: Send + Sync + 'static {
    fn create_user<'a>(
        &'a self,
        user: UserRecord,
    ) -> BoxFuture<'a, Result<UserRecord, ServerError>>;
    fn find_user_by_email<'a>(
        &'a self,
        email: &'a str,
    ) -> BoxFuture<'a, Result<Option<UserRecord>, ServerError>>;
    fn find_user_by_id<'a>(
        &'a self,
        id: &'a str,
    ) -> BoxFuture<'a, Result<Option<UserRecord>, ServerError>>;

    fn insert_refresh_token<'a>(
        &'a self,
        token: RefreshTokenRecord,
    ) -> BoxFuture<'a, Result<(), ServerError>>;
    fn find_refresh_by_hash<'a>(
        &'a self,
        token_hash: &'a str,
    ) -> BoxFuture<'a, Result<Option<RefreshTokenRecord>, ServerError>>;
    fn revoke_refresh_token<'a>(
        &'a self,
        id: &'a str,
        revoked_at: u64,
    ) -> BoxFuture<'a, Result<(), ServerError>>;

    fn insert_api_key<'a>(&'a self, key: ApiKeyRecord) -> BoxFuture<'a, Result<(), ServerError>>;
    fn find_api_key_by_hash<'a>(
        &'a self,
        key_hash: &'a str,
    ) -> BoxFuture<'a, Result<Option<ApiKeyRecord>, ServerError>>;
    fn find_api_key_by_id<'a>(
        &'a self,
        id: &'a str,
    ) -> BoxFuture<'a, Result<Option<ApiKeyRecord>, ServerError>>;
    fn list_api_keys_for_user<'a>(
        &'a self,
        user_id: &'a str,
    ) -> BoxFuture<'a, Result<Vec<ApiKeyRecord>, ServerError>>;
    fn revoke_api_key<'a>(
        &'a self,
        id: &'a str,
        revoked_at: u64,
    ) -> BoxFuture<'a, Result<(), ServerError>>;
}
