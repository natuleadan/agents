//! Core authentication service: Argon2id passwords, HS256 JWTs, and SHA-256 opaque keys.

pub mod crypto;

use std::{sync::Arc, time::Duration};

use agents_protocol::{ApiKeyMetadata, ApiKeyScope, TokenPair, UserClaims};
use ironic::prelude::*;
use serde_json::json;

use crate::{
    infra::{config::ServerConfig, error::ServerError},
    modules::{
        audit::services::AuditService,
        auth::{
            dto::{
                CreateApiKeyDto, CreatedApiKeyResponse, LoginDto, RefreshDto, RegisterDto,
                RevokeApiKeyResponse,
            },
            entities::{ApiKeyRecord, AuthenticatedActor, RefreshTokenRecord, UserRecord},
            repositories::AuthStore,
        },
    },
};

pub use crate::infra::state::StateStore;
pub use crypto::{
    compute_hmac_hex, compute_hmac_sha256, constant_time_eq_hex, decode_jwt,
    generate_opaque_secret, hash_password, now_secs, sha256_hex, sign_jwt, verify_password,
};

/// Manages user identity, credential hashing/verification, and token/key issuance.
#[derive(Injectable)]
pub struct AuthService {
    store: Arc<AuthStore>,
    config: Arc<ServerConfig>,
    state: Arc<StateStore>,
    audit: Arc<AuditService>,
}

impl AuthService {
    /// Seeds an initial admin user if `AGENTS_ADMIN_EMAIL` and `AGENTS_ADMIN_INITIAL_PASSWORD` are set.
    ///
    /// # Errors
    ///
    /// Returns [`ServerError`] when repository access or password hashing fails.
    pub async fn seed_admin_if_configured(&self) -> Result<(), ServerError> {
        let Some(email) = self.config.admin_email.as_deref() else {
            return Ok(());
        };
        let Some(password) = self.config.admin_initial_password.as_deref() else {
            return Ok(());
        };
        if self.store.0.find_user_by_email(email).await?.is_some() {
            return Ok(());
        }
        self.register_with_role(
            email,
            password,
            "admin",
            vec![
                ApiKeyScope::Read,
                ApiKeyScope::Write,
                ApiKeyScope::Exec,
                ApiKeyScope::Admin,
            ],
        )
        .await?;
        Ok(())
    }

    /// Registers a standard user with Argon2id password hashing.
    ///
    /// # Errors
    ///
    /// Returns [`ServerError::Conflict`] on duplicate email or [`ServerError::Internal`] on hashing/storage error.
    pub async fn register(&self, dto: &RegisterDto) -> Result<AuthenticatedActor, ServerError> {
        let user = self
            .register_with_role(
                &dto.email,
                &dto.password,
                "user",
                vec![ApiKeyScope::Read, ApiKeyScope::Write, ApiKeyScope::Exec],
            )
            .await?;

        self.audit
            .record(
                Some(&user.id),
                "auth.register",
                "user",
                Some(&user.id),
                "success",
                json!({ "email": user.email }),
            )
            .await?;

        Ok(AuthenticatedActor {
            user_id: user.id,
            email: user.email,
            scopes: user.scopes,
            auth_method: "password".to_owned(),
        })
    }

    /// Registers a user with explicit role/scopes (for bootstrap seeding or tests).
    ///
    /// # Errors
    ///
    /// Returns [`ServerError`] on conflict or hashing failure.
    pub async fn register_with_role(
        &self,
        email: &str,
        password: &str,
        role: &str,
        scopes: Vec<ApiKeyScope>,
    ) -> Result<UserRecord, ServerError> {
        let normalized_email = email.trim().to_ascii_lowercase();
        let password_hash = hash_password(password)?;
        let user = UserRecord {
            id: uuid::Uuid::new_v4().to_string(),
            email: normalized_email,
            password_hash,
            role: role.to_owned(),
            scopes,
            created_at: now_secs(),
        };
        self.store.0.create_user(user).await
    }

    /// Authenticates email + password via Argon2id and issues JWT access + opaque refresh tokens.
    ///
    /// # Errors
    ///
    /// Returns [`ServerError::RateLimited`] or [`ServerError::Unauthorized`].
    pub async fn login(&self, dto: &LoginDto) -> Result<TokenPair, ServerError> {
        let email = dto.email.trim().to_ascii_lowercase();
        let rl = self
            .state
            .0
            .check_rate_limit(
                &format!("login:{email}"),
                self.config.rate_limit_max_requests.min(20),
                Duration::from_secs(self.config.rate_limit_window_secs),
            )
            .await?;
        if !rl.allowed {
            return Err(ServerError::RateLimited(
                "too many login attempts; retry later".into(),
            ));
        }

        let user = match self.store.0.find_user_by_email(&email).await? {
            Some(user) => user,
            None => {
                // Dummy Argon2id verification to mitigate user-enumeration timing side channels.
                let _ = verify_password(
                    &dto.password,
                    "$argon2id$v=19$m=19456,t=2,p=1$AAAAAAAAAAAAAAAAAAAAAA$AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA",
                );
                let _ = self
                    .audit
                    .record(
                        None,
                        "auth.login",
                        "user",
                        None,
                        "denied",
                        json!({ "reason": "invalid_credentials" }),
                    )
                    .await;
                return Err(ServerError::Unauthorized(
                    "invalid email or password".into(),
                ));
            }
        };

        if !verify_password(&dto.password, &user.password_hash) {
            let _ = self
                .audit
                .record(
                    Some(&user.id),
                    "auth.login",
                    "user",
                    Some(&user.id),
                    "denied",
                    json!({ "reason": "invalid_password" }),
                )
                .await;
            return Err(ServerError::Unauthorized(
                "invalid email or password".into(),
            ));
        }

        let tokens = self.issue_token_pair(&user).await?;
        self.audit
            .record(
                Some(&user.id),
                "auth.login",
                "user",
                Some(&user.id),
                "success",
                json!({}),
            )
            .await?;
        Ok(tokens)
    }

    /// Rotates a revocable opaque refresh token into a fresh [`TokenPair`].
    ///
    /// # Errors
    ///
    /// Returns [`ServerError::Unauthorized`] when missing, expired, revoked, or mismatched.
    pub async fn refresh(&self, dto: &RefreshDto) -> Result<TokenPair, ServerError> {
        let digest = sha256_hex(&dto.refresh_token);
        let record = self
            .store
            .0
            .find_refresh_by_hash(&digest)
            .await?
            .ok_or_else(|| ServerError::Unauthorized("invalid refresh token".into()))?;

        if !constant_time_eq_hex(&record.token_hash, &digest) {
            return Err(ServerError::Unauthorized("invalid refresh token".into()));
        }
        let now = now_secs();
        if record.revoked_at.is_some() || record.expires_at <= now {
            return Err(ServerError::Unauthorized(
                "refresh token expired or revoked".into(),
            ));
        }

        let user = self
            .store
            .0
            .find_user_by_id(&record.user_id)
            .await?
            .ok_or_else(|| ServerError::Unauthorized("account not found".into()))?;

        self.store.0.revoke_refresh_token(&record.id, now).await?;
        self.issue_token_pair(&user).await
    }

    /// Revokes an active opaque refresh token.
    ///
    /// # Errors
    ///
    /// Returns [`ServerError`] when persistence fails.
    pub async fn logout(&self, dto: &RefreshDto) -> Result<(), ServerError> {
        let digest = sha256_hex(&dto.refresh_token);
        if let Some(record) = self.store.0.find_refresh_by_hash(&digest).await? {
            self.store
                .0
                .revoke_refresh_token(&record.id, now_secs())
                .await?;
            let _ = self
                .audit
                .record(
                    Some(&record.user_id),
                    "auth.logout",
                    "refresh_token",
                    Some(&record.id),
                    "success",
                    json!({}),
                )
                .await;
        }
        Ok(())
    }

    /// Mints a revocable SHA-256-hashed programmatic API key for `actor`.
    ///
    /// # Errors
    ///
    /// Returns [`ServerError`] on scope violation or persistence failure.
    pub async fn create_api_key(
        &self,
        actor: &AuthenticatedActor,
        dto: &CreateApiKeyDto,
    ) -> Result<CreatedApiKeyResponse, ServerError> {
        for scope in &dto.scopes {
            if !actor.has_scope(scope) {
                return Err(ServerError::Forbidden(format!(
                    "cannot mint API key with unheld scope `{}`",
                    scope.as_str()
                )));
            }
        }
        let plaintext_key = generate_opaque_secret("ag_ak");
        let prefix = plaintext_key.chars().take(12).collect::<String>();
        let key_hash = sha256_hex(&plaintext_key);
        let now = now_secs();
        let expires_at = dto.expires_in_seconds.map(|ttl| now.saturating_add(ttl));
        let record = ApiKeyRecord {
            id: uuid::Uuid::new_v4().to_string(),
            user_id: actor.user_id.clone(),
            label: dto.label.trim().to_owned(),
            prefix: prefix.clone(),
            key_hash,
            scopes: dto.scopes.clone(),
            created_at: now,
            expires_at,
            revoked_at: None,
        };
        self.store.0.insert_api_key(record.clone()).await?;
        self.audit
            .record(
                Some(&actor.user_id),
                "auth.api_key.create",
                "api_key",
                Some(&record.id),
                "success",
                json!({ "prefix": prefix }),
            )
            .await?;

        Ok(CreatedApiKeyResponse {
            metadata: ApiKeyMetadata {
                key_id: record.id,
                label: record.label,
                prefix: record.prefix,
                scopes: record.scopes,
                created_at: record.created_at,
                expires_at: record.expires_at,
                revoked_at: record.revoked_at,
            },
            plaintext_key,
        })
    }

    /// Lists non-secret metadata for API keys owned by `actor`.
    ///
    /// # Errors
    ///
    /// Returns [`ServerError`] on storage error.
    pub async fn list_api_keys(
        &self,
        actor: &AuthenticatedActor,
    ) -> Result<Vec<ApiKeyMetadata>, ServerError> {
        let records = self.store.0.list_api_keys_for_user(&actor.user_id).await?;
        Ok(records
            .into_iter()
            .map(|r| ApiKeyMetadata {
                key_id: r.id,
                label: r.label,
                prefix: r.prefix,
                scopes: r.scopes,
                created_at: r.created_at,
                expires_at: r.expires_at,
                revoked_at: r.revoked_at,
            })
            .collect())
    }

    /// Revokes an API key after enforcing object ownership (BOLA defense).
    ///
    /// # Errors
    ///
    /// Returns [`ServerError::NotFound`] or [`ServerError::Forbidden`].
    pub async fn revoke_api_key(
        &self,
        actor: &AuthenticatedActor,
        key_id: &str,
    ) -> Result<RevokeApiKeyResponse, ServerError> {
        let record = self
            .store
            .0
            .find_api_key_by_id(key_id)
            .await?
            .ok_or_else(|| ServerError::NotFound(format!("API key `{key_id}` not found")))?;

        if record.user_id != actor.user_id && !actor.has_scope(&ApiKeyScope::Admin) {
            return Err(ServerError::Forbidden(
                "cannot revoke an API key owned by another user".into(),
            ));
        }
        let revoked_at = now_secs();
        self.store.0.revoke_api_key(key_id, revoked_at).await?;
        self.audit
            .record(
                Some(&actor.user_id),
                "auth.api_key.revoke",
                "api_key",
                Some(key_id),
                "success",
                json!({}),
            )
            .await?;

        Ok(RevokeApiKeyResponse {
            key_id: key_id.to_owned(),
            revoked_at,
        })
    }

    /// Verifies either a Bearer JWT or an opaque API key (`ag_ak_...`).
    ///
    /// # Errors
    ///
    /// Returns [`ServerError::Unauthorized`] when invalid, expired, or revoked.
    pub async fn authenticate_token(
        &self,
        raw_token: &str,
    ) -> Result<AuthenticatedActor, ServerError> {
        let token = raw_token.trim();
        if token.is_empty() {
            return Err(ServerError::Unauthorized(
                "missing authentication token".into(),
            ));
        }
        if token.starts_with("ag_ak_") {
            return self.verify_api_key(token).await;
        }
        let claims = self.verify_jwt(token)?;
        Ok(AuthenticatedActor {
            user_id: claims.subject,
            email: claims.email,
            scopes: claims.scopes,
            auth_method: "jwt".to_owned(),
        })
    }

    /// Verifies an HS256 JWT access token and returns [`UserClaims`].
    ///
    /// # Errors
    ///
    /// Returns [`ServerError::Unauthorized`] when signature or expiration checks fail.
    pub fn verify_jwt(&self, token: &str) -> Result<UserClaims, ServerError> {
        decode_jwt(&self.config.jwt_secret, token)
    }

    async fn verify_api_key(&self, plaintext_key: &str) -> Result<AuthenticatedActor, ServerError> {
        let digest = sha256_hex(plaintext_key);
        let record = self
            .store
            .0
            .find_api_key_by_hash(&digest)
            .await?
            .ok_or_else(|| ServerError::Unauthorized("invalid API key".into()))?;

        if !constant_time_eq_hex(&record.key_hash, &digest) {
            return Err(ServerError::Unauthorized("invalid API key".into()));
        }
        let now = now_secs();
        if record.revoked_at.is_some() || record.expires_at.is_some_and(|exp| exp <= now) {
            return Err(ServerError::Unauthorized(
                "API key expired or revoked".into(),
            ));
        }
        let user = self
            .store
            .0
            .find_user_by_id(&record.user_id)
            .await?
            .ok_or_else(|| ServerError::Unauthorized("API key owner not found".into()))?;

        Ok(AuthenticatedActor {
            user_id: user.id,
            email: user.email,
            scopes: record.scopes,
            auth_method: "api_key".to_owned(),
        })
    }

    async fn issue_token_pair(&self, user: &UserRecord) -> Result<TokenPair, ServerError> {
        let now = now_secs();
        let access_exp = now.saturating_add(self.config.jwt_access_ttl_secs);
        let access_token = sign_jwt(
            &self.config.jwt_secret,
            &user.id,
            &user.email,
            &user.scopes,
            now,
            access_exp,
        )?;

        let refresh_secret = generate_opaque_secret("ag_rt");
        let refresh_record = RefreshTokenRecord {
            id: uuid::Uuid::new_v4().to_string(),
            user_id: user.id.clone(),
            token_hash: sha256_hex(&refresh_secret),
            scopes: user.scopes.clone(),
            created_at: now,
            expires_at: now.saturating_add(self.config.refresh_ttl_secs),
            revoked_at: None,
        };
        self.store.0.insert_refresh_token(refresh_record).await?;

        Ok(TokenPair {
            access_token,
            refresh_token: refresh_secret,
            token_type: "Bearer".to_owned(),
            expires_in_seconds: self.config.jwt_access_ttl_secs,
        })
    }
}
