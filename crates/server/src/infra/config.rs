//! Environment-driven server configuration with fail-closed production checks.

use std::fmt;

use argon2::password_hash::rand_core::{OsRng, RngCore};
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Runtime environment profile controlling strict security validation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
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

/// Infrastructure operating mode separating sovereign local stacks from managed cloud stacks.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum InfrastructureMode {
    #[default]
    SelfHosted,
    ManagedCloud,
}

impl InfrastructureMode {
    #[must_use]
    pub fn parse(raw: &str) -> Self {
        match raw.trim().to_ascii_lowercase().as_str() {
            "managed-cloud" | "cloud" | "third-party" | "managed" => Self::ManagedCloud,
            _ => Self::SelfHosted,
        }
    }

    #[must_use]
    pub const fn is_managed_cloud(self) -> bool {
        matches!(self, Self::ManagedCloud)
    }
}

/// Configuration validation failures.
#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("AGENTS_JWT_SECRET must be at least 32 bytes in production")]
    WeakJwtSecret,
    #[error("DATABASE_URL must be configured in production")]
    MissingDatabaseUrl,
    #[error("Managed cloud mode requires `{0}` to be configured")]
    MissingCloudRequirement(&'static str),
    #[error("Conflicting topology configuration: {0}")]
    MixedTopology(&'static str),
    #[error("Invalid numeric configuration value for `{0}`")]
    InvalidNumber(&'static str),
}

/// Runtime configuration loaded from environment variables.
#[derive(Clone)]
pub struct ServerConfig {
    pub env: EnvironmentMode,
    pub infrastructure_mode: InfrastructureMode,
    pub bind_addr: String,
    pub public_url: String,
    pub trusted_origins: Vec<String>,
    pub jwt_secret: String,
    pub cluster_secret: String,
    pub jwt_access_ttl_secs: u64,
    pub refresh_ttl_secs: u64,
    pub admin_email: Option<String>,
    pub admin_initial_password: Option<String>,
    pub database_url: Option<String>,
    pub database_max_connections: u32,
    pub valkey_url: Option<String>,
    pub nats_url: Option<String>,
    pub nats_cluster_name: String,
    pub upstash_redis_url: Option<String>,
    pub qstash_url: Option<String>,
    pub qstash_token: Option<String>,
    pub qstash_current_signing_key: Option<String>,
    pub qstash_next_signing_key: Option<String>,
    pub upstash_vector_rest_url: Option<String>,
    pub upstash_vector_rest_token: Option<String>,
    pub upstash_blob_token: Option<String>,
    pub upstash_box_token: Option<String>,
    pub deepinfra_token: Option<String>,
    pub deepseek_api_key: Option<String>,
    pub ai_gateway_api_key: Option<String>,
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
            .field("infrastructure_mode", &self.infrastructure_mode)
            .field("bind_addr", &self.bind_addr)
            .field("public_url", &self.public_url)
            .field("trusted_origins", &self.trusted_origins)
            .field("jwt_secret", &"[REDACTED]")
            .field("cluster_secret", &"[REDACTED]")
            .field("jwt_access_ttl_secs", &self.jwt_access_ttl_secs)
            .field("refresh_ttl_secs", &self.refresh_ttl_secs)
            .field("admin_email", &self.admin_email)
            .field("admin_initial_password", &"[REDACTED]")
            .field("database_url", &"[REDACTED]")
            .field("database_max_connections", &self.database_max_connections)
            .field("valkey_url", &"[REDACTED]")
            .field("nats_url", &self.nats_url)
            .field("nats_cluster_name", &self.nats_cluster_name)
            .field("upstash_redis_url", &"[REDACTED]")
            .field("qstash_url", &self.qstash_url)
            .field("qstash_token", &"[REDACTED]")
            .field("qstash_current_signing_key", &"[REDACTED]")
            .field("qstash_next_signing_key", &"[REDACTED]")
            .field("upstash_vector_rest_url", &self.upstash_vector_rest_url)
            .field("upstash_vector_rest_token", &"[REDACTED]")
            .field("upstash_blob_token", &"[REDACTED]")
            .field("upstash_box_token", &"[REDACTED]")
            .field("deepinfra_token", &"[REDACTED]")
            .field("deepseek_api_key", &"[REDACTED]")
            .field("ai_gateway_api_key", &"[REDACTED]")
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
        let infrastructure_mode =
            InfrastructureMode::parse(&env_or_local("INFRASTRUCTURE_MODE", "self-hosted"));

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

        let upstash_redis_url = opt("REDIS_URL");
        let qstash_url = opt("QSTASH_URL");
        let qstash_token = opt("QSTASH_TOKEN");
        let qstash_current_signing_key = opt("QSTASH_CURRENT_SIGNING_KEY");
        let qstash_next_signing_key = opt("QSTASH_NEXT_SIGNING_KEY");
        let upstash_vector_rest_url = opt("UPSTASH_VECTOR_REST_URL");
        let upstash_vector_rest_token = opt("UPSTASH_VECTOR_REST_TOKEN");
        let upstash_blob_token = opt("UPSTASH_BLOB_TOKEN");
        let upstash_box_token = opt("UPSTASH_BOX_TOKEN");
        let deepinfra_token = opt("DEEPINFRA_TOKEN");
        let deepseek_api_key = opt("DEEPSEEK_API_KEY");
        let ai_gateway_api_key = opt("AI_GATEWAY_API_KEY");

        if infrastructure_mode.is_managed_cloud() {
            if database_url.is_none() {
                return Err(ConfigError::MissingCloudRequirement("DATABASE_URL"));
            }
            if upstash_redis_url.is_none() && opt("VALKEY_URL").is_none() {
                return Err(ConfigError::MissingCloudRequirement("REDIS_URL"));
            }
            if qstash_url.is_none() {
                return Err(ConfigError::MissingCloudRequirement("QSTASH_URL"));
            }
            if env.is_production() && opt("NATS_URL").is_some() {
                return Err(ConfigError::MixedTopology(
                    "managed-cloud mode cannot run with sovereign NATS_URL in production",
                ));
            }
        } else if env.is_production() && (opt("QSTASH_URL").is_some() && opt("NATS_URL").is_some())
        {
            return Err(ConfigError::MixedTopology(
                "self-hosted mode cannot mix sovereign NATS_URL with cloud QSTASH_URL in production",
            ));
        }

        let trusted_origins = env_or_local("AGENTS_TRUSTED_ORIGINS", "http://127.0.0.1:5173")
            .split(',')
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(ToOwned::to_owned)
            .collect();

        Ok(Self {
            env,
            infrastructure_mode,
            bind_addr: env_or_local("AGENTS_BIND_ADDR", "127.0.0.1:8080"),
            public_url: env_or_local("AGENTS_PUBLIC_URL", "http://127.0.0.1:8080"),
            trusted_origins,
            jwt_secret,
            cluster_secret: opt("AGENTS_CLUSTER_SECRET")
                .or_else(|| opt("BOXER_CLUSTER_SECRET"))
                .unwrap_or_else(|| "dev-boxer-cluster-secret".to_owned()),
            jwt_access_ttl_secs: parse_u64("AGENTS_JWT_ACCESS_TTL_SECS", 900)?,
            refresh_ttl_secs: parse_u64("AGENTS_REFRESH_TTL_SECS", 2_592_000)?,
            admin_email: opt("AGENTS_ADMIN_EMAIL"),
            admin_initial_password: opt("AGENTS_ADMIN_INITIAL_PASSWORD"),
            database_url,
            database_max_connections: parse_u32("DATABASE_MAX_CONNECTIONS", 20)?,
            valkey_url: opt("VALKEY_URL"),
            nats_url: opt("NATS_URL"),
            nats_cluster_name: env_or_local("NATS_CLUSTER_NAME", "agents-cluster"),
            upstash_redis_url,
            qstash_url,
            qstash_token,
            qstash_current_signing_key,
            qstash_next_signing_key,
            upstash_vector_rest_url,
            upstash_vector_rest_token,
            upstash_blob_token,
            upstash_box_token,
            deepinfra_token,
            deepseek_api_key,
            ai_gateway_api_key,
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
            infrastructure_mode: InfrastructureMode::SelfHosted,
            bind_addr: "127.0.0.1:0".to_owned(),
            public_url: "http://127.0.0.1".to_owned(),
            trusted_origins: vec!["http://127.0.0.1".to_owned()],
            jwt_secret: generate_ephemeral_secret(),
            cluster_secret: "dev-boxer-cluster-secret".to_owned(),
            jwt_access_ttl_secs: 900,
            refresh_ttl_secs: 86_400,
            admin_email: None,
            admin_initial_password: None,
            database_url: None,
            database_max_connections: 5,
            valkey_url: None,
            nats_url: None,
            nats_cluster_name: "agents-test".to_owned(),
            upstash_redis_url: None,
            qstash_url: None,
            qstash_token: None,
            qstash_current_signing_key: None,
            qstash_next_signing_key: None,
            upstash_vector_rest_url: None,
            upstash_vector_rest_token: None,
            upstash_blob_token: None,
            upstash_box_token: None,
            deepinfra_token: None,
            deepseek_api_key: None,
            ai_gateway_api_key: None,
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

    #[test]
    fn managed_cloud_mode_fails_closed_when_cloud_keys_missing() {
        let missing_db = ServerConfig::from_lookup(|k| match k {
            "INFRASTRUCTURE_MODE" => Some("managed-cloud".to_owned()),
            _ => None,
        });
        assert!(matches!(
            missing_db,
            Err(ConfigError::MissingCloudRequirement("DATABASE_URL"))
        ));

        let missing_redis = ServerConfig::from_lookup(|k| match k {
            "INFRASTRUCTURE_MODE" => Some("managed-cloud".to_owned()),
            "DATABASE_URL" => Some("postgres://user:pass@ep-test.neon.tech/db".to_owned()),
            _ => None,
        });
        assert!(matches!(
            missing_redis,
            Err(ConfigError::MissingCloudRequirement("REDIS_URL"))
        ));

        let missing_qstash = ServerConfig::from_lookup(|k| match k {
            "INFRASTRUCTURE_MODE" => Some("managed-cloud".to_owned()),
            "DATABASE_URL" => Some("postgres://user:pass@ep-test.neon.tech/db".to_owned()),
            "REDIS_URL" => Some("rediss://default:token@skunk.upstash.io:6379".to_owned()),
            _ => None,
        });
        assert!(matches!(
            missing_qstash,
            Err(ConfigError::MissingCloudRequirement("QSTASH_URL"))
        ));

        let valid = ServerConfig::from_lookup(|k| match k {
            "INFRASTRUCTURE_MODE" => Some("managed-cloud".to_owned()),
            "DATABASE_URL" => Some("postgres://user:pass@ep-test.neon.tech/db".to_owned()),
            "REDIS_URL" => Some("rediss://default:token@skunk.upstash.io:6379".to_owned()),
            "QSTASH_URL" => Some("https://qstash.upstash.io".to_owned()),
            _ => None,
        });
        assert!(valid.is_ok());

        let mixed_cloud = ServerConfig::from_lookup(|k| match k {
            "AGENTS_ENV" => Some("production".to_owned()),
            "INFRASTRUCTURE_MODE" => Some("managed-cloud".to_owned()),
            "AGENTS_JWT_SECRET" => Some("a-very-long-production-secret-over-32-bytes".to_owned()),
            "DATABASE_URL" => Some("postgres://user:pass@ep-test.neon.tech/db".to_owned()),
            "REDIS_URL" => Some("rediss://default:token@skunk.upstash.io:6379".to_owned()),
            "QSTASH_URL" => Some("https://qstash.upstash.io".to_owned()),
            "NATS_URL" => Some("nats://127.0.0.1:4222".to_owned()),
            _ => None,
        });
        assert!(matches!(mixed_cloud, Err(ConfigError::MixedTopology(_))));
    }
}
