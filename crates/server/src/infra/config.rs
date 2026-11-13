//! Environment-driven server configuration with fail-closed production checks.

use std::fmt;

use argon2::password_hash::rand_core::{OsRng, RngCore};
use thiserror::Error;

/// Runtime environment profile controlling strict security validation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum EnvironmentMode {
    #[default]
    Development,
    Test,
    Production,
}

impl EnvironmentMode {
    #[must_use]
    pub fn parse(raw: &str) -> Self {
        match raw.trim().to_ascii_lowercase().as_str() {
            "production" | "prod" => Self::Production,
            "test" | "testing" => Self::Test,
            _ => Self::Development,
        }
    }

    #[must_use]
    pub const fn is_production(self) -> bool {
        matches!(self, Self::Production)
    }
}

/// Configuration validation failures.
#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("AGENTS_JWT_SECRET must be at least 32 bytes in production")]
    WeakJwtSecret,
    #[error("DATABASE_URL must be configured in production")]
    MissingDatabaseUrl,
    #[error("Invalid numeric configuration value for `{0}`")]
    InvalidNumber(&'static str),
}

/// Runtime configuration loaded from environment variables.
#[derive(Clone)]
pub struct ServerConfig {
    pub env: EnvironmentMode,
    pub bind_addr: String,
    pub public_url: String,
    pub trusted_origins: Vec<String>,
    pub jwt_secret: String,
    pub jwt_access_ttl_secs: u64,
    pub refresh_ttl_secs: u64,
    pub admin_email: Option<String>,
    pub admin_initial_password: Option<String>,
    pub database_url: Option<String>,
    pub database_max_connections: u32,
    pub valkey_url: Option<String>,
    pub nats_url: Option<String>,
    pub nats_cluster_name: String,
    pub rate_limit_max_requests: u64,
    pub rate_limit_window_secs: u64,
    pub max_concurrent_global: usize,
    pub max_concurrent_session: usize,
    pub audit_retention_secs: u64,
}

impl fmt::Debug for ServerConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ServerConfig")
            .field("env", &self.env)
            .field("bind_addr", &self.bind_addr)
            .field("public_url", &self.public_url)
            .field("trusted_origins", &self.trusted_origins)
            .field("jwt_secret", &"[REDACTED]")
            .field("jwt_access_ttl_secs", &self.jwt_access_ttl_secs)
            .field("refresh_ttl_secs", &self.refresh_ttl_secs)
            .field("admin_email", &self.admin_email)
            .field(
                "admin_initial_password",
                &self.admin_initial_password.as_ref().map(|_| "[REDACTED]"),
            )
            .field(
                "database_url",
                &self.database_url.as_ref().map(|_| "[CONFIGURED]"),
            )
            .field("database_max_connections", &self.database_max_connections)
            .field(
                "valkey_url",
                &self.valkey_url.as_ref().map(|_| "[CONFIGURED]"),
            )
            .field("nats_url", &self.nats_url.as_ref().map(|_| "[CONFIGURED]"))
            .field("nats_cluster_name", &self.nats_cluster_name)
            .field("rate_limit_max_requests", &self.rate_limit_max_requests)
            .field("rate_limit_window_secs", &self.rate_limit_window_secs)
            .field("max_concurrent_global", &self.max_concurrent_global)
            .field("max_concurrent_session", &self.max_concurrent_session)
            .field("audit_retention_secs", &self.audit_retention_secs)
            .finish()
    }
}

impl ServerConfig {
    /// Loads server configuration from process environment variables.
    ///
    /// # Errors
    ///
    /// Returns [`ConfigError`] when production secrets are missing/weak or numeric fields fail to parse.
    pub fn from_env() -> Result<Self, ConfigError> {
        Self::from_lookup(|key| std::env::var(key).ok())
    }

    /// Loads server configuration from an explicit lookup closure (deterministic in tests).
    ///
    /// # Errors
    ///
    /// Returns [`ConfigError`] when production secrets are missing/weak or numeric fields fail to parse.
    pub fn from_lookup<F>(lookup: F) -> Result<Self, ConfigError>
    where
        F: Fn(&str) -> Option<String>,
    {
        let opt = |k: &str| {
            lookup(k)
                .map(|s| s.trim().to_owned())
                .filter(|s| !s.is_empty())
        };
        let env_or_local = |k: &str, def: &str| opt(k).unwrap_or_else(|| def.to_owned());
        let parse_u64 = |k: &'static str, def: u64| match opt(k) {
            Some(raw) => raw
                .parse::<u64>()
                .map_err(|_| ConfigError::InvalidNumber(k)),
            None => Ok(def),
        };
        let parse_u32 = |k: &'static str, def: u32| match opt(k) {
            Some(raw) => raw
                .parse::<u32>()
                .map_err(|_| ConfigError::InvalidNumber(k)),
            None => Ok(def),
        };
        let parse_usize = |k: &'static str, def: usize| match opt(k) {
            Some(raw) => raw
                .parse::<usize>()
                .map_err(|_| ConfigError::InvalidNumber(k)),
            None => Ok(def),
        };

        let env = EnvironmentMode::parse(&env_or_local("AGENTS_ENV", "development"));
        let raw_secret = opt("AGENTS_JWT_SECRET");
        let jwt_secret = match raw_secret {
            Some(secret) if secret.len() >= 32 => secret,
            Some(_) if env.is_production() => return Err(ConfigError::WeakJwtSecret),
            None if env.is_production() => return Err(ConfigError::WeakJwtSecret),
            Some(secret) => secret,
            None => generate_ephemeral_secret(),
        };

        let database_url = opt("DATABASE_URL");
        if env.is_production() && database_url.is_none() {
            return Err(ConfigError::MissingDatabaseUrl);
        }

        let trusted_origins = env_or_local("AGENTS_TRUSTED_ORIGINS", "http://127.0.0.1:5173")
            .split(',')
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(ToOwned::to_owned)
            .collect();

        Ok(Self {
            env,
            bind_addr: env_or_local("AGENTS_BIND_ADDR", "127.0.0.1:8080"),
            public_url: env_or_local("AGENTS_PUBLIC_URL", "http://127.0.0.1:8080"),
            trusted_origins,
            jwt_secret,
            jwt_access_ttl_secs: parse_u64("AGENTS_JWT_ACCESS_TTL_SECS", 900)?,
            refresh_ttl_secs: parse_u64("AGENTS_REFRESH_TTL_SECS", 2_592_000)?,
            admin_email: opt("AGENTS_ADMIN_EMAIL"),
            admin_initial_password: opt("AGENTS_ADMIN_INITIAL_PASSWORD"),
            database_url,
            database_max_connections: parse_u32("DATABASE_MAX_CONNECTIONS", 20)?,
            valkey_url: opt("VALKEY_URL"),
            nats_url: opt("NATS_URL"),
            nats_cluster_name: env_or_local("NATS_CLUSTER_NAME", "agents-cluster"),
            rate_limit_max_requests: parse_u64("AGENTS_RATE_LIMIT_MAX", 120)?,
            rate_limit_window_secs: parse_u64("AGENTS_RATE_LIMIT_WINDOW_SECS", 60)?,
            max_concurrent_global: parse_usize("AGENTS_MAX_CONCURRENT_GLOBAL", 64)?,
            max_concurrent_session: parse_usize("AGENTS_MAX_CONCURRENT_SESSION", 4)?,
            audit_retention_secs: parse_u64("AGENTS_AUDIT_RETENTION_SECS", 2_592_000)?,
        })
    }

    /// Creates an isolated hermetic configuration for unit and integration tests.
    #[must_use]
    pub fn for_tests() -> Self {
        Self {
            env: EnvironmentMode::Test,
            bind_addr: "127.0.0.1:0".to_owned(),
            public_url: "http://127.0.0.1".to_owned(),
            trusted_origins: vec!["http://127.0.0.1".to_owned()],
            jwt_secret: generate_ephemeral_secret(),
            jwt_access_ttl_secs: 900,
            refresh_ttl_secs: 86_400,
            admin_email: None,
            admin_initial_password: None,
            database_url: None,
            database_max_connections: 5,
            valkey_url: None,
            nats_url: None,
            nats_cluster_name: "agents-test".to_owned(),
            rate_limit_max_requests: 60,
            rate_limit_window_secs: 60,
            max_concurrent_global: 32,
            max_concurrent_session: 2,
            audit_retention_secs: 86_400,
        }
    }
}

/// Generates a cryptographically random 32-byte hex secret for ephemeral dev/test runs.
#[must_use]
pub fn generate_ephemeral_secret() -> String {
    let mut bytes = [0_u8; 32];
    OsRng.fill_bytes(&mut bytes);
    hex::encode(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn production_mode_fails_closed_without_strong_secret_and_database() {
        let weak = ServerConfig::from_lookup(|k| match k {
            "AGENTS_ENV" => Some("production".to_owned()),
            "AGENTS_JWT_SECRET" => Some("too-short".to_owned()),
            _ => None,
        });
        assert!(matches!(weak, Err(ConfigError::WeakJwtSecret)));

        let missing_db = ServerConfig::from_lookup(|k| match k {
            "AGENTS_ENV" => Some("production".to_owned()),
            "AGENTS_JWT_SECRET" => Some("a-very-long-production-secret-over-32-bytes".to_owned()),
            _ => None,
        });
        assert!(matches!(missing_db, Err(ConfigError::MissingDatabaseUrl)));

        let valid = ServerConfig::from_lookup(|k| match k {
            "AGENTS_ENV" => Some("production".to_owned()),
            "AGENTS_JWT_SECRET" => Some("a-very-long-production-secret-over-32-bytes".to_owned()),
            "DATABASE_URL" => Some("postgres://user:pass@localhost/agents".to_owned()),
            _ => None,
        });
        assert!(valid.is_ok());
    }
}
