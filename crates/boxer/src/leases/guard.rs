//! Zero-unsafe RAII `CleanupDropGuard` guaranteeing sandbox process-group,
//! container, and ephemeral route destruction on normal exit, abort, timeout, or panic.

use std::{
    process::Command,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

use tracing::warn;

/// Safe RAII drop guard running a cleanup closure on scope exit unless disarmed.
pub struct CleanupDropGuard<T, F>
where
    F: FnOnce(T),
{
    state: Option<T>,
    cleanup: Option<F>,
}

impl<T, F> CleanupDropGuard<T, F>
where
    F: FnOnce(T),
{
    /// Arms a new guard owning `state` and executing `cleanup(state)` on drop.
    #[must_use]
    pub fn new(state: T, cleanup: F) -> Self {
        Self {
            state: Some(state),
            cleanup: Some(cleanup),
        }
    }

    /// Disarms the guard and returns the owned state without invoking cleanup.
    pub fn dismiss(mut self) -> Option<T> {
        self.cleanup.take();
        self.state.take()
    }

    /// Runs cleanup immediately and disarms the guard.
    pub fn run_now(mut self) {
        if let (Some(state), Some(cleanup)) = (self.state.take(), self.cleanup.take()) {
            cleanup(state);
        }
    }
}

impl<T, F> Drop for CleanupDropGuard<T, F>
where
    F: FnOnce(T),
{
    fn drop(&mut self) {
        if let (Some(state), Some(cleanup)) = (self.state.take(), self.cleanup.take()) {
            cleanup(state);
        }
    }
}

/// Active sandbox resources registered for guaranteed teardown inside [`CleanupDropGuard`].
#[derive(Debug, Clone)]
pub struct SandboxResources {
    pub task_id: String,
    pub process_group_id: Option<u32>,
    pub container_name: Option<String>,
    pub preview_route_file: Option<std::path::PathBuf>,
    pub cleaned: Arc<AtomicBool>,
}

impl SandboxResources {
    #[must_use]
    pub fn new(task_id: impl Into<String>) -> Self {
        Self {
            task_id: task_id.into(),
            process_group_id: None,
            container_name: None,
            preview_route_file: None,
            cleaned: Arc::new(AtomicBool::new(false)),
        }
    }

    /// Executes deterministic idempotent teardown of process groups, Docker containers,
    /// and ephemeral Traefik preview files.
    pub fn cleanup_sync(&self) {
        if self.cleaned.swap(true, Ordering::SeqCst) {
            return;
        }

        if let Some(pgid) = self.process_group_id {
            terminate_process_group_sync(pgid);
        }

        if let Some(container) = &self.container_name
            && !container.trim().is_empty()
        {
            let _ = Command::new("docker")
                .args(["rm", "-f", container])
                .output();
        }

        if let Some(route_file) = &self.preview_route_file
            && route_file.exists()
            && let Err(err) = std::fs::remove_file(route_file)
        {
            warn!(
                task_id = %self.task_id,
                path = %route_file.display(),
                error = %err,
                "failed to remove preview route file during drop cleanup"
            );
        }
    }
}

/// Sends `SIGTERM` to `-pgid`, waits briefly if alive, then escalates to `SIGKILL`.
pub fn terminate_process_group_sync(pgid: u32) {
    if pgid <= 1 {
        return;
    }
    let target = format!("-{pgid}");
    let _ = Command::new("kill")
        .args(["-s", "TERM", "--", &target])
        .output();
    std::thread::sleep(std::time::Duration::from_millis(40));
    let _ = Command::new("kill")
        .args(["-s", "KILL", "--", &target])
        .output();
}
