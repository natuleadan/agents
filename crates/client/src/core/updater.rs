use std::collections::BTreeMap;

use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
use ed25519_dalek::{Signature, VerifyingKey};
use semver::Version;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use thiserror::Error;

/// Errors raised while verifying release manifests, binary downloads, or `.nshpkg` OTA bundles.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum UpdateError {
    #[error("insecure update url `{0}`: HTTPS required except localhost/127.0.0.1")]
    InsecureUrl(String),
    #[error("missing or invalid ed25519 public key")]
    InvalidPublicKey,
    #[error("missing or invalid ed25519 signature")]
    InvalidSignature,
    #[error("manifest app id mismatch: expected `{expected}`, got `{actual}`")]
    AppIdMismatch { expected: String, actual: String },
    #[error("invalid app id `{0}`: must be a single path segment")]
    UnsafeAppId(String),
    #[error("manifest json error: {0}")]
    Json(String),
    #[error("invalid semver `{0}`")]
    InvalidVersion(String),
    #[error("archive size mismatch: expected {expected} bytes, got {actual}")]
    SizeMismatch { expected: u64, actual: u64 },
    #[error("archive sha256 mismatch: expected {expected}, got {actual}")]
    Sha256Mismatch { expected: String, actual: String },
    #[error("package refused: signed by untrusted key `{0}`")]
    UntrustedSigner(String),
    #[error("package refused: release key certificate expired at {expires_at} (now {now})")]
    ExpiredCertificate { expires_at: u64, now: u64 },
    #[error("package refused: revoked ({0})")]
    Revoked(String),
    #[error("package refused: build {candidate} is not newer than floor {floor}")]
    RollbackRejected { candidate: u64, floor: u64 },
    #[error("package refused: needs runtime {required}, host has {current}")]
    UnsupportedRuntime { required: String, current: String },
    #[error("io error: {0}")]
    Io(String),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlatformAsset {
    pub url: String,
    pub sha256: String,
    pub size: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OtaPackageAsset {
    pub version: String,
    pub build: u64,
    pub url: String,
    pub sha256: String,
    pub size: u64,
    pub min_runtime: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReleaseManifest {
    pub id: String,
    pub version: String,
    pub notes: String,
    #[serde(default)]
    pub prerelease: bool,
    #[serde(default)]
    pub platforms: BTreeMap<String, PlatformAsset>,
    #[serde(default)]
    pub package: Option<OtaPackageAsset>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UpdateKind {
    OtaBundle,
    AppBinary,
    StoreLink,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AvailableUpdate {
    pub kind: UpdateKind,
    pub version: String,
    pub build: Option<u64>,
    pub url: String,
    pub sha256: Option<String>,
    pub size: Option<u64>,
    pub notes: String,
    pub can_install: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpdaterConfig {
    pub app_id: String,
    pub current_version: String,
    pub current_build: u64,
    pub current_runtime: String,
    pub platform_key: String,
    pub update_public_key_b64: Option<String>,
    pub trusted_root_pubkeys_b64: Vec<String>,
    pub highest_accepted_build: u64,
    pub can_self_install: bool,
    pub allow_prerelease: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReleaseKeyCertificate {
    pub root_pubkey_b64: String,
    pub release_pubkey_b64: String,
    pub app_pattern: String,
    pub expires_at_epoch: u64,
    pub root_sig_b64: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SignedOtaPackage {
    pub app_id: String,
    pub version: String,
    pub build: u64,
    pub min_runtime: String,
    pub signed_at_epoch: u64,
    pub payload_sha256: String,
    pub payload_b64: String,
    pub certificate: ReleaseKeyCertificate,
    pub release_sig_b64: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RevocationList {
    pub sequence: u64,
    #[serde(default)]
    pub revoked_key_ids: Vec<String>,
    #[serde(default)]
    pub revoked_package_sha256: Vec<String>,
}

/// Validates update transport URLs (`https://`, or loopback `http://127.0.0.1` / `localhost`).
pub fn validate_update_url(raw: &str) -> Result<(), UpdateError> {
    if raw.starts_with("https://")
        || raw.starts_with("http://127.0.0.1")
        || raw.starts_with("http://localhost")
        || (!raw.contains("://") && !raw.is_empty())
    {
        Ok(())
    } else {
        Err(UpdateError::InsecureUrl(raw.to_owned()))
    }
}

/// Computes lowercase SHA-256 hex digest.
#[must_use]
pub fn sha256_hex(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    hex::encode(hasher.finalize())
}

/// Computes 16-hex-char NSH key id (`sha256(pubkey)[..8]`).
pub fn key_id_from_b64(pubkey_b64: &str) -> Result<String, UpdateError> {
    let raw = BASE64
        .decode(pubkey_b64.trim())
        .map_err(|_| UpdateError::InvalidPublicKey)?;
    Ok(sha256_hex(&raw).chars().take(16).collect())
}

fn parse_verifying_key(pubkey_b64: &str) -> Result<VerifyingKey, UpdateError> {
    let bytes = BASE64
        .decode(pubkey_b64.trim())
        .map_err(|_| UpdateError::InvalidPublicKey)?;
    let arr: [u8; 32] = bytes
        .try_into()
        .map_err(|_| UpdateError::InvalidPublicKey)?;
    VerifyingKey::from_bytes(&arr).map_err(|_| UpdateError::InvalidPublicKey)
}

fn parse_signature(sig_b64: &str) -> Result<Signature, UpdateError> {
    let bytes = BASE64
        .decode(sig_b64.trim())
        .map_err(|_| UpdateError::InvalidSignature)?;
    let arr: [u8; 64] = bytes
        .try_into()
        .map_err(|_| UpdateError::InvalidSignature)?;
    Ok(Signature::from_bytes(&arr))
}

/// Verifies raw Ed25519 signature (`latest.json.sig` against `latest.json` bytes).
pub fn verify_ed25519(pubkey_b64: &str, sig_b64: &str, msg: &[u8]) -> Result<(), UpdateError> {
    let vk = parse_verifying_key(pubkey_b64)?;
    let sig = parse_signature(sig_b64)?;
    vk.verify_strict(msg, &sig)
        .map_err(|_| UpdateError::InvalidSignature)
}

fn ensure_safe_app_id(app_id: &str) -> Result<(), UpdateError> {
    if app_id.is_empty()
        || app_id == "."
        || app_id == ".."
        || app_id.contains(['/', '\\', ':', '\0'])
    {
        return Err(UpdateError::UnsafeAppId(app_id.to_owned()));
    }
    Ok(())
}

fn pattern_covers_app(pattern: &str, app_id: &str) -> bool {
    if pattern == "*" {
        return true;
    }
    if let Some(prefix) = pattern.strip_suffix('*') {
        return app_id.starts_with(prefix);
    }
    pattern == app_id
}

/// Canonical byte message covered by a root certificate signature.
#[must_use]
pub fn certificate_signing_message(cert: &ReleaseKeyCertificate) -> Vec<u8> {
    format!(
        "nsh-cert-v1\n{}\n{}\n{}",
        cert.release_pubkey_b64.trim(),
        cert.app_pattern.trim(),
        cert.expires_at_epoch
    )
    .into_bytes()
}

/// Canonical byte message covered by the release key signature inside `.nshpkg`.
#[must_use]
pub fn package_signing_message(pkg: &SignedOtaPackage) -> Vec<u8> {
    format!(
        "nsh-pkg-v1\n{}\n{}\n{}\n{}\n{}\n{}",
        pkg.app_id,
        pkg.version,
        pkg.build,
        pkg.min_runtime,
        pkg.signed_at_epoch,
        pkg.payload_sha256
    )
    .into_bytes()
}

/// Checks a signed `ReleaseManifest` (`latest.json` + optional `latest.json.sig`) for OTA or app upgrades.
pub fn check_manifest(
    cfg: &UpdaterConfig,
    manifest_bytes: &[u8],
    manifest_sig_b64: Option<&str>,
) -> Result<Option<AvailableUpdate>, UpdateError> {
    ensure_safe_app_id(&cfg.app_id)?;
    if let Some(pk) = &cfg.update_public_key_b64 {
        let sig = manifest_sig_b64.ok_or(UpdateError::InvalidSignature)?;
        verify_ed25519(pk, sig, manifest_bytes)?;
    }

    let manifest: ReleaseManifest =
        serde_json::from_slice(manifest_bytes).map_err(|e| UpdateError::Json(e.to_string()))?;

    if manifest.id != cfg.app_id {
        return Err(UpdateError::AppIdMismatch {
            expected: cfg.app_id.clone(),
            actual: manifest.id,
        });
    }
    if manifest.prerelease && !cfg.allow_prerelease {
        return Ok(None);
    }

    if let Some(pkg) = &manifest.package {
        let required_rt = Version::parse(&pkg.min_runtime)
            .map_err(|_| UpdateError::InvalidVersion(pkg.min_runtime.clone()))?;
        let host_rt = Version::parse(&cfg.current_runtime)
            .map_err(|_| UpdateError::InvalidVersion(cfg.current_runtime.clone()))?;
        let floor = cfg.current_build.max(cfg.highest_accepted_build);
        if pkg.build > floor && required_rt <= host_rt {
            validate_update_url(&pkg.url)?;
            return Ok(Some(AvailableUpdate {
                kind: UpdateKind::OtaBundle,
                version: pkg.version.clone(),
                build: Some(pkg.build),
                url: pkg.url.clone(),
                sha256: Some(pkg.sha256.clone()),
                size: Some(pkg.size),
                notes: manifest.notes,
                can_install: true,
            }));
        }
    }

    let current_ver = Version::parse(&cfg.current_version)
        .map_err(|_| UpdateError::InvalidVersion(cfg.current_version.clone()))?;
    let remote_ver = Version::parse(&manifest.version)
        .map_err(|_| UpdateError::InvalidVersion(manifest.version.clone()))?;
    if remote_ver <= current_ver {
        return Ok(None);
    }

    if let Some(asset) = manifest.platforms.get(&cfg.platform_key) {
        validate_update_url(&asset.url)?;
        let kind = if cfg.can_self_install {
            UpdateKind::AppBinary
        } else {
            UpdateKind::StoreLink
        };
        return Ok(Some(AvailableUpdate {
            kind,
            version: manifest.version,
            build: None,
            url: asset.url.clone(),
            sha256: Some(asset.sha256.clone()),
            size: Some(asset.size),
            notes: manifest.notes,
            can_install: cfg.can_self_install,
        }));
    }

    Ok(None)
}

/// Verifies downloaded archive or `.nshpkg` size and SHA-256 checksum.
pub fn verify_download_bytes(
    bytes: &[u8],
    expected_sha256: &str,
    expected_size: u64,
) -> Result<(), UpdateError> {
    let actual_size = bytes.len() as u64;
    if actual_size != expected_size {
        return Err(UpdateError::SizeMismatch {
            expected: expected_size,
            actual: actual_size,
        });
    }
    let actual_hash = sha256_hex(bytes);
    if !actual_hash.eq_ignore_ascii_case(expected_sha256) {
        return Err(UpdateError::Sha256Mismatch {
            expected: expected_sha256.to_owned(),
            actual: actual_hash,
        });
    }
    Ok(())
}

/// Performs the full 8-step NSH verification on a `.nshpkg` OTA bundle.
pub fn verify_signed_nshpkg(
    cfg: &UpdaterConfig,
    pkg_bytes: &[u8],
    revocations: &RevocationList,
    now_epoch: u64,
) -> Result<SignedOtaPackage, UpdateError> {
    ensure_safe_app_id(&cfg.app_id)?;
    let pkg: SignedOtaPackage =
        serde_json::from_slice(pkg_bytes).map_err(|e| UpdateError::Json(e.to_string()))?;
    ensure_safe_app_id(&pkg.app_id)?;
    if pkg.app_id != cfg.app_id {
        return Err(UpdateError::AppIdMismatch {
            expected: cfg.app_id.clone(),
            actual: pkg.app_id,
        });
    }

    // 1. Trusted root verifies the release certificate
    let root_trusted = cfg
        .trusted_root_pubkeys_b64
        .iter()
        .any(|r| r.trim() == pkg.certificate.root_pubkey_b64.trim());
    if !root_trusted {
        return Err(UpdateError::UntrustedSigner(
            pkg.certificate.root_pubkey_b64,
        ));
    }
    let cert_msg = certificate_signing_message(&pkg.certificate);
    verify_ed25519(
        &pkg.certificate.root_pubkey_b64,
        &pkg.certificate.root_sig_b64,
        &cert_msg,
    )?;

    // 2. Expiry & scope
    if pkg.certificate.expires_at_epoch < now_epoch {
        return Err(UpdateError::ExpiredCertificate {
            expires_at: pkg.certificate.expires_at_epoch,
            now: now_epoch,
        });
    }
    if !pattern_covers_app(&pkg.certificate.app_pattern, &pkg.app_id) {
        return Err(UpdateError::AppIdMismatch {
            expected: pkg.certificate.app_pattern,
            actual: pkg.app_id,
        });
    }

    // 3. Key and package revocation checks
    let rel_key_id = key_id_from_b64(&pkg.certificate.release_pubkey_b64)?;
    if revocations.revoked_key_ids.contains(&rel_key_id) {
        return Err(UpdateError::Revoked(format!("release key {rel_key_id}")));
    }
    let pkg_hash = sha256_hex(pkg_bytes);
    if revocations.revoked_package_sha256.contains(&pkg_hash) {
        return Err(UpdateError::Revoked(format!("package {pkg_hash}")));
    }

    // 4. Release key signature & payload hash check
    let pkg_msg = package_signing_message(&pkg);
    verify_ed25519(
        &pkg.certificate.release_pubkey_b64,
        &pkg.release_sig_b64,
        &pkg_msg,
    )?;
    let payload = BASE64
        .decode(pkg.payload_b64.trim())
        .map_err(|e| UpdateError::Json(e.to_string()))?;
    let actual_payload_hash = sha256_hex(&payload);
    if actual_payload_hash != pkg.payload_sha256 {
        return Err(UpdateError::Sha256Mismatch {
            expected: pkg.payload_sha256,
            actual: actual_payload_hash,
        });
    }

    // 5. Runtime and rollback floor
    let required_rt = Version::parse(&pkg.min_runtime)
        .map_err(|_| UpdateError::InvalidVersion(pkg.min_runtime.clone()))?;
    let host_rt = Version::parse(&cfg.current_runtime)
        .map_err(|_| UpdateError::InvalidVersion(cfg.current_runtime.clone()))?;
    if required_rt > host_rt {
        return Err(UpdateError::UnsupportedRuntime {
            required: pkg.min_runtime,
            current: cfg.current_runtime.clone(),
        });
    }
    let floor = cfg.current_build.max(cfg.highest_accepted_build);
    if pkg.build <= floor {
        return Err(UpdateError::RollbackRejected {
            candidate: pkg.build,
            floor,
        });
    }

    Ok(pkg)
}
