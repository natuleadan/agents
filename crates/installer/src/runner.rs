use std::future::Future;
use std::pin::Pin;
use std::process::Stdio;
use std::sync::{Arc, Mutex};

use tokio::io::AsyncWriteExt;
use tokio::process::Command;

use crate::doctor::redact_secrets;
use crate::error::InstallerError;
use crate::planner::{ExecutionPlan, ScriptStep};

pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// Captured stdout/stderr output from executing one external script step.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandOutput {
    pub exit_code: i32,
    pub stdout: String,
    pub stderr: String,
}

/// Abstract executor enabling both live host/SSH execution and deterministic dry-run/test recording.
pub trait CommandExecutor: Send + Sync {
    fn execute_step<'a>(
        &'a self,
        step: &'a ScriptStep,
    ) -> BoxFuture<'a, Result<CommandOutput, InstallerError>>;
}

/// Real local `bash` and remote `ssh` executor passing arguments as safe argv items.
#[derive(Debug, Default, Clone, Copy)]
pub struct SystemExecutor;

impl CommandExecutor for SystemExecutor {
    fn execute_step<'a>(
        &'a self,
        step: &'a ScriptStep,
    ) -> BoxFuture<'a, Result<CommandOutput, InstallerError>> {
        Box::pin(async move {
            tracing::info!(
                script = %step.script_name,
                remote = step.ssh_target.is_some(),
                args = %redact_secrets(&step.args.join(" ")),
                "executing provisioning script step"
            );

            match &step.ssh_target {
                None => run_local_script(step).await,
                Some(target) => run_ssh_script(target, step).await,
            }
        })
    }
}

async fn run_local_script(step: &ScriptStep) -> Result<CommandOutput, InstallerError> {
    let mut cmd = Command::new("bash");
    cmd.arg(&step.script_path);
    cmd.args(&step.args);
    let out = cmd.output().await?;
    let exit_code = out.status.code().unwrap_or(1);
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    if !out.status.success() {
        return Err(InstallerError::Execution {
            script: step.script_name.clone(),
            code: out.status.code(),
            message: redact_secrets(&stderr),
        });
    }
    Ok(CommandOutput {
        exit_code,
        stdout,
        stderr,
    })
}

async fn run_ssh_script(target: &str, step: &ScriptStep) -> Result<CommandOutput, InstallerError> {
    let script_bytes = tokio::fs::read(&step.script_path).await?;
    let mut cmd = Command::new("ssh");
    cmd.arg("-o").arg("BatchMode=yes");
    cmd.arg("-o").arg("IdentitiesOnly=yes");
    cmd.arg("-o").arg("StrictHostKeyChecking=accept-new");
    if let Some(key) = &step.ssh_key_path {
        cmd.arg("-i").arg(key);
    }
    cmd.arg(target);
    cmd.arg("bash").arg("-s").arg("--");
    cmd.args(&step.args);
    cmd.stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    let mut child = cmd.spawn()?;
    if let Some(mut stdin) = child.stdin.take() {
        stdin.write_all(&script_bytes).await?;
        stdin.shutdown().await?;
    }
    let out = child.wait_with_output().await?;
    let exit_code = out.status.code().unwrap_or(1);
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    if !out.status.success() {
        return Err(InstallerError::Execution {
            script: step.script_name.clone(),
            code: out.status.code(),
            message: redact_secrets(&stderr),
        });
    }
    Ok(CommandOutput {
        exit_code,
        stdout,
        stderr,
    })
}

/// Deterministic in-memory executor capturing executed steps for unit/integration tests.
#[derive(Clone, Default)]
pub struct RecordingExecutor {
    recorded: Arc<Mutex<Vec<ScriptStep>>>,
    canned_stdout: Arc<Mutex<String>>,
}

impl RecordingExecutor {
    #[must_use]
    pub fn with_stdout(stdout: impl Into<String>) -> Self {
        Self {
            recorded: Arc::new(Mutex::new(Vec::new())),
            canned_stdout: Arc::new(Mutex::new(stdout.into())),
        }
    }

    #[must_use]
    pub fn recorded_steps(&self) -> Vec<ScriptStep> {
        self.recorded.lock().map(|g| g.clone()).unwrap_or_default()
    }
}

impl CommandExecutor for RecordingExecutor {
    fn execute_step<'a>(
        &'a self,
        step: &'a ScriptStep,
    ) -> BoxFuture<'a, Result<CommandOutput, InstallerError>> {
        Box::pin(async move {
            if let Ok(mut guard) = self.recorded.lock() {
                guard.push(step.clone());
            }
            let stdout = self
                .canned_stdout
                .lock()
                .map(|g| g.clone())
                .unwrap_or_default();
            Ok(CommandOutput {
                exit_code: 0,
                stdout,
                stderr: String::new(),
            })
        })
    }
}

/// Runs every [`ScriptStep`] in an [`ExecutionPlan`] sequentially, halting on first failure.
///
/// # Errors
///
/// Propagates [`InstallerError`] if any script step exits non-zero.
pub async fn run_plan(
    plan: &ExecutionPlan,
    executor: &dyn CommandExecutor,
) -> Result<Vec<CommandOutput>, InstallerError> {
    let mut outputs = Vec::with_capacity(plan.steps.len());
    for step in &plan.steps {
        outputs.push(executor.execute_step(step).await?);
    }
    Ok(outputs)
}
