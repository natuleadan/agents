//! Two-axis `Runtime x Isolation` sandbox drivers, policy checks, and drivers.

pub mod docker;
pub mod host;
pub mod policy;
pub mod remote;

use std::{future::Future, path::PathBuf, pin::Pin, sync::Arc};

use agents_protocol::{ExecOptions, RuntimeKind, SandboxProfile, TaskStreamEvent};
use tokio::sync::{mpsc, watch};

use crate::{config::BoxerConfig, error::BoxerError};

pub use docker::{DockerCommandSpec, DockerRuntimeDriver};
pub use host::HostRuntimeDriver;
pub use policy::{build_sanitized_env, resolve_confined_workdir, validate_runtime_isolation};
pub use remote::{RemoteExecDescriptor, RemoteVpsRuntimeDriver};

pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// Specification for executing a command inside an isolated sandbox.
#[derive(Debug, Clone)]
pub struct SandboxExecRequest {
    pub task_id: String,
    pub session_id: String,
    pub command: String,
    pub profile: SandboxProfile,
    pub exec: ExecOptions,
    pub proxy_url: Option<String>,
}

/// Captured outcome emitted by a sandbox driver upon exit, timeout, or cancellation.
#[derive(Debug, Clone)]
pub struct SandboxExecOutput {
    pub exit_code: i32,
    pub stdout: String,
    pub stderr: String,
    pub timed_out: bool,
    pub cancelled: bool,
    pub working_dir: PathBuf,
}

/// Abstract two-axis runtime execution substrate.
pub trait SandboxDriver: Send + Sync + 'static {
    fn execute<'a>(
        &'a self,
        req: SandboxExecRequest,
        stream_tx: Option<mpsc::Sender<TaskStreamEvent>>,
        cancel_rx: watch::Receiver<bool>,
    ) -> BoxFuture<'a, Result<SandboxExecOutput, BoxerError>>;
}

/// Selects the runtime driver strictly matching [`SandboxProfile`] without silent fallback.
///
/// # Errors
///
/// Returns [`BoxerError`] when the two-axis profile is invalid or unsupported on this node.
pub fn build_sandbox_driver(
    config: &BoxerConfig,
    profile: &SandboxProfile,
) -> Result<Arc<dyn SandboxDriver>, BoxerError> {
    validate_runtime_isolation(profile, config.strict_native_isolation)?;
    match &profile.runtime {
        RuntimeKind::Host => Ok(Arc::new(HostRuntimeDriver::new(
            config.workspace_root.clone(),
            config.strict_native_isolation,
        ))),
        RuntimeKind::Docker => Ok(Arc::new(DockerRuntimeDriver::new(
            config.workspace_root.clone(),
            config.sandbox_image.clone(),
        ))),
        RuntimeKind::RemoteVps => Ok(Arc::new(RemoteVpsRuntimeDriver::new(
            config.workspace_root.clone(),
        ))),
        RuntimeKind::ThirdParty(provider) => Err(BoxerError::SandboxProfile(
            agents_protocol::SandboxProfileError::ModeRuntimeMismatch,
        )
        .into_context(format!(
            "third-party provider `{}` runs via client third-party mode, not Boxer daemon",
            provider.as_str()
        ))),
        RuntimeKind::Unknown(_) => Err(BoxerError::IsolationViolation(
            "unknown runtime cannot be provisioned".into(),
        )),
    }
}

trait BoxerErrorContext {
    fn into_context(self, detail: String) -> Self;
}

impl BoxerErrorContext for BoxerError {
    fn into_context(self, detail: String) -> Self {
        BoxerError::IsolationViolation(detail)
    }
}
