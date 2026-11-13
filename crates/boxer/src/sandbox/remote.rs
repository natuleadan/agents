//! Remote VPS sandbox execution descriptor and fail-closed VLAN tunnel checks.

use std::path::PathBuf;

use agents_protocol::TaskStreamEvent;
use tokio::sync::{mpsc, watch};

use crate::{
    error::BoxerError,
    sandbox::{
        BoxFuture, SandboxDriver, SandboxExecOutput, SandboxExecRequest,
        policy::{resolve_confined_workdir, validate_runtime_isolation},
    },
};

/// Validated remote VPS execution envelope transmitted over private VLAN.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoteExecDescriptor {
    pub task_id: String,
    pub gateway_url: Option<String>,
    pub isolation: String,
    pub vlan_only: bool,
}

/// Dispatches sandbox workloads to provisioned remote VPS Boxer nodes.
#[derive(Debug, Clone)]
pub struct RemoteVpsRuntimeDriver {
    workspace_root: PathBuf,
}

impl RemoteVpsRuntimeDriver {
    #[must_use]
    pub fn new(workspace_root: PathBuf) -> Self {
        Self { workspace_root }
    }

    /// Validates remote runtime requirements (forbidding `IsolationKind::None`).
    ///
    /// # Errors
    ///
    /// Returns [`BoxerError::IsolationViolation`] if profile or gateway policy fails.
    pub fn build_descriptor(
        &self,
        req: &SandboxExecRequest,
    ) -> Result<RemoteExecDescriptor, BoxerError> {
        validate_runtime_isolation(&req.profile, false)?;
        let _ = resolve_confined_workdir(
            &self.workspace_root,
            &req.session_id,
            req.exec.working_dir.as_deref(),
        )?;
        Ok(RemoteExecDescriptor {
            task_id: req.task_id.clone(),
            gateway_url: req.profile.gateway_url.clone(),
            isolation: req.profile.isolation.as_str().to_owned(),
            vlan_only: true,
        })
    }
}

impl SandboxDriver for RemoteVpsRuntimeDriver {
    fn execute<'a>(
        &'a self,
        req: SandboxExecRequest,
        _stream_tx: Option<mpsc::Sender<TaskStreamEvent>>,
        _cancel_rx: watch::Receiver<bool>,
    ) -> BoxFuture<'a, Result<SandboxExecOutput, BoxerError>> {
        Box::pin(async move {
            let descriptor = self.build_descriptor(&req)?;
            Err(BoxerError::ExecutionFailed(format!(
                "remote_vps transport dispatch for `{}` requires active VLAN tunnel (isolation={})",
                descriptor.task_id, descriptor.isolation
            )))
        })
    }
}
