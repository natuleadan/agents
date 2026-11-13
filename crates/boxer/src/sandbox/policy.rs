//! Fail-closed `Runtime x Isolation` validation, minimal env allowlisting,
//! secret scrubbing, and confined VFS workspace path resolution.

use std::{
    collections::BTreeMap,
    path::{Component, Path, PathBuf},
    process::Command,
};

use agents_protocol::{ExecOptions, IsolationKind, RuntimeKind, SandboxProfile};

use crate::error::BoxerError;

const DEFAULT_SAFE_PATH: &str = "/usr/local/bin:/usr/bin:/bin";

const BLOCKED_ENV_NAMES: &[&str] = &[
    "AGENTS_JWT_SECRET",
    "AGENTS_CREDENTIAL_MASTER_KEY",
    "AGENTS_ADMIN_INITIAL_PASSWORD",
    "DATABASE_URL",
    "VALKEY_URL",
    "NATS_NKEY_SEED",
    "NATS_JWT",
    "OPENAI_API_KEY",
    "ANTHROPIC_API_KEY",
    "DEEPINFRA_API_KEY",
    "UPSTASH_BOX_API_KEY",
    "DAYTONA_API_KEY",
    "INSTABOX_API_KEY",
    "VPS_PROVIDER_API_KEY",
    "AWS_SECRET_ACCESS_KEY",
    "AWS_SESSION_TOKEN",
    "GITHUB_TOKEN",
    "GH_TOKEN",
    "BOXER_WORKER_TOKEN",
];

const BLOCKED_PATH_SEGMENTS: &[&str] = &[".ssh", ".env", ".netrc", ".gitcookies", "docker.sock"];

/// Validates two-axis `Runtime x Isolation` invariants strictly fail-closed.
///
/// # Errors
///
/// Rejects invalid combinations (`Host + Openshell`), un-opted `Host + Native`,
/// missing Openshell gateway URLs, or missing OS confinement tools when strict.
pub fn validate_runtime_isolation(
    profile: &SandboxProfile,
    strict_native_isolation: bool,
) -> Result<(), BoxerError> {
    profile.validate()?;

    if profile.isolation == IsolationKind::Openshell
        && profile
            .gateway_url
            .as_deref()
            .map(str::trim)
            .is_none_or(str::is_empty)
    {
        return Err(BoxerError::IsolationViolation(
            "openshell isolation requires a non-empty gateway_url".into(),
        ));
    }

    if matches!(profile.runtime, RuntimeKind::RemoteVps) && profile.isolation == IsolationKind::None
    {
        return Err(BoxerError::IsolationViolation(
            "remote_vps runtime forbids isolation=none; select native or openshell".into(),
        ));
    }

    if strict_native_isolation
        && profile.isolation == IsolationKind::Native
        && !has_native_confinement_binary()
    {
        return Err(BoxerError::IsolationViolation(
            "strict native isolation requested, but neither `bwrap` nor `unshare` is available"
                .into(),
        ));
    }

    Ok(())
}

/// Checks if OS-level namespace/filesystem confinement (`bwrap` or `unshare`) exists.
#[must_use]
pub fn has_native_confinement_binary() -> bool {
    binary_exists("bwrap") || binary_exists("unshare")
}

#[must_use]
pub fn binary_exists(bin: &str) -> bool {
    Command::new("which")
        .arg(bin)
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// Constructs a clean, minimal environment map scrubbed of all host secrets.
///
/// Only explicit safe locale/path defaults plus allowed non-secret variables from
/// `exec.env_allowlist` and supervisor proxy addresses (never raw tokens) are passed.
#[must_use]
pub fn build_sanitized_env(
    exec: &ExecOptions,
    proxy_url: Option<&str>,
) -> BTreeMap<String, String> {
    let mut env = BTreeMap::new();
    env.insert("PATH".to_owned(), DEFAULT_SAFE_PATH.to_owned());
    env.insert("LANG".to_owned(), "C.UTF-8".to_owned());
    env.insert("LC_ALL".to_owned(), "C.UTF-8".to_owned());
    env.insert("TERM".to_owned(), "xterm-256color".to_owned());

    for var_name in &exec.env_allowlist {
        let trimmed = var_name.trim();
        if trimmed.is_empty() || is_sensitive_env_name(trimmed) {
            continue;
        }
        if let Ok(val) = std::env::var(trimmed) {
            env.insert(trimmed.to_owned(), val);
        }
    }

    if exec.network_egress
        && let Some(url) = proxy_url
    {
        env.insert("AGENTS_CREDENTIAL_PROXY".to_owned(), url.to_owned());
        env.insert("HTTP_PROXY".to_owned(), url.to_owned());
        env.insert("HTTPS_PROXY".to_owned(), url.to_owned());
    }

    env
}

/// Returns true if an environment variable name looks like a credential or secret.
#[must_use]
pub fn is_sensitive_env_name(name: &str) -> bool {
    let upper = name.to_ascii_uppercase();
    if BLOCKED_ENV_NAMES.contains(&upper.as_str()) {
        return true;
    }
    upper.ends_with("_SECRET")
        || upper.ends_with("_KEY")
        || upper.ends_with("_TOKEN")
        || upper.ends_with("_PASSWORD")
        || upper.starts_with("AGENTS_")
}

/// Resolves and canonicalizes a session-scoped workspace path, failing closed
/// on absolute escapes, `..` traversal, symlinks leaving root, or restricted filenames.
///
/// # Errors
///
/// Returns [`BoxerError::IsolationViolation`] if any path constraint is violated.
pub fn resolve_confined_workdir(
    workspace_root: &Path,
    session_id: &str,
    requested_subdir: Option<&str>,
) -> Result<PathBuf, BoxerError> {
    if session_id.trim().is_empty()
        || session_id.contains('/')
        || session_id.contains('\\')
        || session_id.contains("..")
    {
        return Err(BoxerError::IsolationViolation(format!(
            "invalid session workspace id `{session_id}`"
        )));
    }

    let session_dir = workspace_root.join(session_id);
    std::fs::create_dir_all(&session_dir)?;
    let canonical_root = std::fs::canonicalize(&session_dir)?;

    let Some(raw_sub) = requested_subdir.map(str::trim).filter(|s| !s.is_empty()) else {
        return Ok(canonical_root);
    };

    let rel = Path::new(raw_sub);
    if rel.is_absolute() {
        return Err(BoxerError::IsolationViolation(format!(
            "absolute working directory `{raw_sub}` is forbidden inside sandbox"
        )));
    }

    for comp in rel.components() {
        match comp {
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                return Err(BoxerError::IsolationViolation(format!(
                    "path traversal `{raw_sub}` escapes confined workspace"
                )));
            }
            Component::Normal(part) => {
                let s = part.to_string_lossy();
                if BLOCKED_PATH_SEGMENTS
                    .iter()
                    .any(|blocked| s.eq_ignore_ascii_case(blocked))
                {
                    return Err(BoxerError::IsolationViolation(format!(
                        "restricted path segment `{s}` cannot be mounted or entered"
                    )));
                }
            }
            Component::CurDir => {}
        }
    }

    let candidate = canonical_root.join(rel);
    std::fs::create_dir_all(&candidate)?;
    let canonical_target = std::fs::canonicalize(&candidate)?;

    if !canonical_target.starts_with(&canonical_root) {
        return Err(BoxerError::IsolationViolation(format!(
            "resolved path `{}` escaped session root `{}`",
            canonical_target.display(),
            canonical_root.display()
        )));
    }

    Ok(canonical_target)
}
