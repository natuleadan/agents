//! Cryptographic primitives for Argon2id password hashing, SHA-256 token digests,
//! constant-time comparisons, and HS256 JWT encoding/decoding.

use std::time::{SystemTime, UNIX_EPOCH};

use agents_protocol::{ApiKeyScope, UserClaims};
use argon2::{
    Argon2, PasswordHash, PasswordHasher, PasswordVerifier,
    password_hash::{
        SaltString,
        rand_core::{OsRng, RngCore},
    },
};
use jsonwebtoken::{Algorithm, DecodingKey, EncodingKey, Header, Validation, decode, encode};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;

use crate::infra::error::ServerError;

#[derive(Debug, Serialize, Deserialize)]
struct JwtClaims {
    sub: String,
    email: String,
    scopes: Vec<ApiKeyScope>,
    iat: u64,
    exp: u64,
    jti: String,
}

/// Signs an HS256 JWT access token for a verified user.
///
/// # Errors
///
/// Returns [`ServerError::Internal`] if JWT encoding fails.
pub fn sign_jwt(
    secret: &str,
    user_id: &str,
    email: &str,
    scopes: &[ApiKeyScope],
    issued_at: u64,
    expires_at: u64,
) -> Result<String, ServerError> {
    let claims = JwtClaims {
        sub: user_id.to_owned(),
        email: email.to_owned(),
        scopes: scopes.to_vec(),
        iat: issued_at,
        exp: expires_at,
        jti: uuid::Uuid::new_v4().to_string(),
    };
    encode(
        &Header::new(Algorithm::HS256),
        &claims,
        &EncodingKey::from_secret(secret.as_bytes()),
    )
    .map_err(|e| ServerError::Internal(format!("JWT signing failed: {e}")))
}

/// Decodes and validates an HS256 JWT access token with zero leeway.
///
/// # Errors
///
/// Returns [`ServerError::Unauthorized`] when signature or expiration checks fail.
pub fn decode_jwt(secret: &str, token: &str) -> Result<UserClaims, ServerError> {
    let mut validation = Validation::new(Algorithm::HS256);
    validation.leeway = 0;
    let data = decode::<JwtClaims>(
        token,
        &DecodingKey::from_secret(secret.as_bytes()),
        &validation,
    )
    .map_err(|err| ServerError::Unauthorized(format!("invalid JWT: {err}")))?;

    Ok(UserClaims {
        subject: data.claims.sub,
        email: data.claims.email,
        scopes: data.claims.scopes,
        issued_at: data.claims.iat,
        expires_at: data.claims.exp,
    })
}

/// Hashes a plaintext password using Argon2id with a CSPRNG salt.
///
/// # Errors
///
/// Returns [`ServerError::Internal`] if password hashing fails.
pub fn hash_password(password: &str) -> Result<String, ServerError> {
    let salt = SaltString::generate(&mut OsRng);
    Argon2::default()
        .hash_password(password.as_bytes(), &salt)
        .map(|hash| hash.to_string())
        .map_err(|e| ServerError::Internal(format!("argon2id hash failed: {e}")))
}

/// Verifies a plaintext password against a stored Argon2id PHC hash.
#[must_use]
pub fn verify_password(password: &str, stored_phc: &str) -> bool {
    let Ok(parsed) = PasswordHash::new(stored_phc) else {
        return false;
    };
    Argon2::default()
        .verify_password(password.as_bytes(), &parsed)
        .is_ok()
}

/// Computes lowercase SHA-256 hex digest for opaque token storage.
#[must_use]
pub fn sha256_hex(secret: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(secret.as_bytes());
    hex::encode(hasher.finalize())
}

/// Constant-time string comparison via `subtle::ConstantTimeEq`.
#[must_use]
pub fn constant_time_eq_hex(a: &str, b: &str) -> bool {
    let ab = a.as_bytes();
    let bb = b.as_bytes();
    ab.len() == bb.len() && bool::from(ab.ct_eq(bb))
}

/// Generates a CSPRNG opaque secret with an identifiable prefix.
#[must_use]
pub fn generate_opaque_secret(prefix: &str) -> String {
    let mut random = [0_u8; 24];
    OsRng.fill_bytes(&mut random);
    format!("{prefix}_{}", hex::encode(random))
}

/// Current UNIX timestamp in seconds.
#[must_use]
pub fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

/// Computes RFC 2104 HMAC-SHA256 for a secret key and message.
#[must_use]
pub fn compute_hmac_sha256(key: &[u8], msg: &[u8]) -> [u8; 32] {
    const BLOCK_SIZE: usize = 64;
    let mut k_block = [0u8; BLOCK_SIZE];
    if key.len() > BLOCK_SIZE {
        let hashed = Sha256::digest(key);
        k_block[..32].copy_from_slice(&hashed);
    } else {
        k_block[..key.len()].copy_from_slice(key);
    }
    let mut o_pad = [0x5cu8; BLOCK_SIZE];
    let mut i_pad = [0x36u8; BLOCK_SIZE];
    for i in 0..BLOCK_SIZE {
        o_pad[i] ^= k_block[i];
        i_pad[i] ^= k_block[i];
    }
    let mut inner = Sha256::new();
    inner.update(i_pad);
    inner.update(msg);
    let inner_hash = inner.finalize();

    let mut outer = Sha256::new();
    outer.update(o_pad);
    outer.update(inner_hash);
    outer.finalize().into()
}

/// Computes lowercase hex-encoded HMAC-SHA256.
#[must_use]
pub fn compute_hmac_hex(key: &[u8], msg: &[u8]) -> String {
    hex::encode(compute_hmac_sha256(key, msg))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;

    #[test]
    fn argon2id_sha256_and_jwt_primitives_behave_securely() {
        let phc = hash_password("correct-horse-battery").unwrap();
        assert!(phc.starts_with("$argon2id$"));
        assert!(verify_password("correct-horse-battery", &phc));
        assert!(!verify_password("wrong-password", &phc));
        assert!(!verify_password("anything", "not-a-valid-phc"));

        let secret = generate_opaque_secret("ag_ak");
        assert!(secret.starts_with("ag_ak_"));
        let digest = sha256_hex(&secret);
        assert_eq!(digest.len(), 64);
        assert!(constant_time_eq_hex(&digest, &sha256_hex(&secret)));
        assert!(!constant_time_eq_hex(&digest, "short"));

        let now = now_secs();
        let jwt = sign_jwt(
            "unit-test-secret-over-32-bytes-long",
            "usr-1",
            "u@example.com",
            &[ApiKeyScope::Read, ApiKeyScope::Exec],
            now,
            now + 60,
        )
        .unwrap();
        let claims = decode_jwt("unit-test-secret-over-32-bytes-long", &jwt).unwrap();
        assert_eq!(claims.subject, "usr-1");
        assert!(decode_jwt("wrong-secret-over-32-bytes-long!!", &jwt).is_err());

        let expired = sign_jwt(
            "unit-test-secret-over-32-bytes-long",
            "usr-1",
            "u@example.com",
            &[ApiKeyScope::Read],
            now.saturating_sub(120),
            now.saturating_sub(60),
        )
        .unwrap();
        assert!(decode_jwt("unit-test-secret-over-32-bytes-long", &expired).is_err());
    }
}
