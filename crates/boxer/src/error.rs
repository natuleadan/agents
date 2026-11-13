//! Typed errors emitted across Boxer configuration, leasing, confinement, and previews.

use agents_protocol::{ArtifactValidationError, PermissionError, SandboxProfileError};
use thiserror::Error;

/// Canonical error enum for the `agents-boxer` sandbox supervisor.
#[derive(Debug, Error)]
pub enum BoxerError {
    #[error("configuration error: {0}")]
    Config(String),

    #[error("invalid sandbox profile: {0}")]
    SandboxProfile(#[from] SandboxProfileError),

    #[error("sandbox isolation violation: {0}")]
    IsolationViolation(String),

    #[error("fencing conflict: {0}")]
    FencingConflict(String),

    #[error("task lease expired: {0}")]
    LeaseExpired(String),

    #[error("sandbox execution failed: {0}")]
    ExecutionFailed(String),

    #[error("task execution cancelled: {0}")]
    Cancelled(String),

    #[error("task execution timed out after {0}s")]
    Timeout(u64),

    #[error("artifact packaging error: {0}")]
    Artifact(String),

    #[error("artifact validation error: {0}")]
    ArtifactValidation(#[from] ArtifactValidationError),

    #[error("permission error: {0}")]
    Permission(#[from] PermissionError),

    #[error("preview route error: {0}")]
    Preview(String),

    #[error("outbound credential proxy error: {0}")]
    Proxy(String),

    #[error("message bus error: {0}")]
    Bus(String),

    #[error("control-plane server API error: {0}")]
    ServerApi(String),

    #[error("i/o error: {0}")]
    Io(#[from] std::io::Error),
}
