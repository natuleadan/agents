//! Complete 8-stage Ironic application and module lifecycle management.

use std::sync::{
    Arc, Mutex, OnceLock,
    atomic::{AtomicBool, AtomicU64, Ordering},
};

use ironic::{
    AfterShutdown, BeforeShutdown, LifecycleFuture, OnApplicationBootstrap, OnApplicationShutdown,
    OnModuleConfigure, OnModuleDestroy, OnModuleInit, OnServerReady, ShutdownSignal,
};
use serde::Serialize;

use super::metrics::DomainMetrics;

/// Snapshot of lifecycle hook counts observed across the application's lifetime.
#[derive(Clone, Debug, Serialize)]
pub struct LifecycleSnapshot {
    /// Number of `OnModuleConfigure` callbacks invoked.
    pub module_configure_count: u64,
    /// Number of `OnModuleInit` callbacks invoked.
    pub module_init_count: u64,
    /// Whether `OnApplicationBootstrap` has completed.
    pub bootstrapped: bool,
    /// Whether `OnServerReady` has fired.
    pub server_ready: bool,
    /// Number of `BeforeShutdown` callbacks invoked.
    pub before_shutdown_count: u64,
    /// Number of `OnApplicationShutdown` callbacks invoked.
    pub application_shutdown_count: u64,
    /// Number of `OnModuleDestroy` callbacks invoked.
    pub module_destroy_count: u64,
    /// Whether `AfterShutdown` has completed.
    pub after_shutdown: bool,
    /// Most recently configured module name, if recorded.
    pub last_module: Option<String>,
    /// Most recent shutdown signal name, if recorded.
    pub last_signal: Option<String>,
}

#[derive(Default)]
struct LifecycleState {
    module_configure_count: AtomicU64,
    module_init_count: AtomicU64,
    bootstrapped: AtomicBool,
    server_ready: AtomicBool,
    before_shutdown_count: AtomicU64,
    application_shutdown_count: AtomicU64,
    module_destroy_count: AtomicU64,
    after_shutdown: AtomicBool,
    last_module: Mutex<Option<String>>,
    last_signal: Mutex<Option<String>>,
}

/// Coordinates all 8 Ironic module and server lifecycle hooks for observability and clean drain.
#[derive(Clone, Default)]
pub struct LifecycleManager {
    state: Arc<LifecycleState>,
}

static LIFECYCLE_MANAGER: OnceLock<LifecycleManager> = OnceLock::new();

impl LifecycleManager {
    /// Returns the process-wide singleton [`LifecycleManager`].
    #[must_use]
    pub fn global() -> &'static Self {
        LIFECYCLE_MANAGER.get_or_init(Self::default)
    }

    /// Returns an atomic point-in-time [`LifecycleSnapshot`].
    #[must_use]
    pub fn snapshot(&self) -> LifecycleSnapshot {
        let last_module = self
            .state
            .last_module
            .lock()
            .map_or(None, |guard| guard.clone());
        let last_signal = self
            .state
            .last_signal
            .lock()
            .map_or(None, |guard| guard.clone());
        LifecycleSnapshot {
            module_configure_count: self.state.module_configure_count.load(Ordering::Relaxed),
            module_init_count: self.state.module_init_count.load(Ordering::Relaxed),
            bootstrapped: self.state.bootstrapped.load(Ordering::Acquire),
            server_ready: self.state.server_ready.load(Ordering::Acquire),
            before_shutdown_count: self.state.before_shutdown_count.load(Ordering::Relaxed),
            application_shutdown_count: self
                .state
                .application_shutdown_count
                .load(Ordering::Relaxed),
            module_destroy_count: self.state.module_destroy_count.load(Ordering::Relaxed),
            after_shutdown: self.state.after_shutdown.load(Ordering::Acquire),
            last_module,
            last_signal,
        }
    }

    fn record_transition(phase: &'static str, detail: Option<&str>) {
        DomainMetrics::global().lifecycle_transitions_total.inc();
        match detail {
            Some(info) => tracing::info!(phase, detail = %info, "ironic lifecycle phase executed"),
            None => tracing::info!(phase, "ironic lifecycle phase executed"),
        }
    }

    fn format_signal(signal: ShutdownSignal) -> String {
        match signal {
            ShutdownSignal::Interrupt => "Interrupt".to_owned(),
            ShutdownSignal::Terminate => "Terminate".to_owned(),
            ShutdownSignal::Custom(name) => name.to_owned(),
            _ => "Other".to_owned(),
        }
    }
}

impl OnModuleConfigure for LifecycleManager {
    fn on_module_configure(&self, module_name: &str) -> LifecycleFuture<'_> {
        self.state
            .module_configure_count
            .fetch_add(1, Ordering::Relaxed);
        if let Ok(mut guard) = self.state.last_module.lock() {
            *guard = Some(module_name.to_owned());
        }
        Self::record_transition("on_module_configure", Some(module_name));
        Box::pin(async { Ok(()) })
    }
}

impl OnModuleInit for LifecycleManager {
    fn on_module_init(&self) -> LifecycleFuture<'_> {
        self.state.module_init_count.fetch_add(1, Ordering::Relaxed);
        Self::record_transition("on_module_init", None);
        Box::pin(async { Ok(()) })
    }
}

impl OnApplicationBootstrap for LifecycleManager {
    fn on_application_bootstrap(&self) -> LifecycleFuture<'_> {
        self.state.bootstrapped.store(true, Ordering::Release);
        Self::record_transition("on_application_bootstrap", None);
        Box::pin(async { Ok(()) })
    }
}

impl OnServerReady for LifecycleManager {
    fn on_server_ready(&self) -> LifecycleFuture<'_> {
        self.state.server_ready.store(true, Ordering::Release);
        Self::record_transition("on_server_ready", None);
        Box::pin(async { Ok(()) })
    }
}

impl BeforeShutdown for LifecycleManager {
    fn before_shutdown(&self, signal: ShutdownSignal) -> LifecycleFuture<'_> {
        self.state
            .before_shutdown_count
            .fetch_add(1, Ordering::Relaxed);
        let name = Self::format_signal(signal);
        if let Ok(mut guard) = self.state.last_signal.lock() {
            *guard = Some(name.clone());
        }
        Self::record_transition("before_shutdown", Some(&name));
        Box::pin(async { Ok(()) })
    }
}

impl OnApplicationShutdown for LifecycleManager {
    fn on_application_shutdown(&self, signal: ShutdownSignal) -> LifecycleFuture<'_> {
        self.state
            .application_shutdown_count
            .fetch_add(1, Ordering::Relaxed);
        let name = Self::format_signal(signal);
        Self::record_transition("on_application_shutdown", Some(&name));
        Box::pin(async { Ok(()) })
    }
}

impl OnModuleDestroy for LifecycleManager {
    fn on_module_destroy(&self) -> LifecycleFuture<'_> {
        self.state
            .module_destroy_count
            .fetch_add(1, Ordering::Relaxed);
        Self::record_transition("on_module_destroy", None);
        Box::pin(async { Ok(()) })
    }
}

impl AfterShutdown for LifecycleManager {
    fn after_shutdown(&self) -> LifecycleFuture<'_> {
        self.state.after_shutdown.store(true, Ordering::Release);
        Self::record_transition("after_shutdown", None);
        Box::pin(async { Ok(()) })
    }
}
