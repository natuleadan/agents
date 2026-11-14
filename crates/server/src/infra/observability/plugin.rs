//! First-party Ironic plugin registration via [`PluginRegistry`].

use ironic::{
    LifecycleDefinition, ModuleDefinitionBuilder, ProviderDefinition, Scope,
    ecosystem::plugins::{Plugin, PluginError, PluginRegistry},
};

use super::{lifecycle::LifecycleManager, metrics::DomainMetrics};

/// First-party Ironic [`Plugin`] registering [`DomainMetrics`], [`LifecycleManager`], and all 8 lifecycle hooks.
#[derive(Clone, Copy, Default)]
pub struct ObservabilityPlugin;

impl Plugin for ObservabilityPlugin {
    fn name(&self) -> &'static str {
        "agents-observability"
    }

    fn version(&self) -> &'static str {
        env!("CARGO_PKG_VERSION")
    }

    fn apply(
        &self,
        module: ModuleDefinitionBuilder,
    ) -> Result<ModuleDefinitionBuilder, PluginError> {
        let metrics = DomainMetrics::global().clone();
        let lifecycle = LifecycleManager::global().clone();
        Ok(module
            .provider(ProviderDefinition::constructor(
                Scope::Singleton,
                Vec::new(),
                move |_| Ok(metrics.clone()),
            ))
            .provider(ProviderDefinition::constructor(
                Scope::Singleton,
                Vec::new(),
                move |_| Ok(lifecycle.clone()),
            ))
            .export::<DomainMetrics>()
            .export::<LifecycleManager>()
            .lifecycle(
                LifecycleDefinition::builder::<LifecycleManager>()
                    .module_configure()
                    .module_init()
                    .application_bootstrap()
                    .server_ready()
                    .before_shutdown()
                    .application_shutdown()
                    .module_destroy()
                    .after_shutdown()
                    .build(),
            ))
    }
}

/// Applies [`ObservabilityPlugin`] through Ironic's [`PluginRegistry`] onto a [`ModuleDefinitionBuilder`].
///
/// # Errors
///
/// Returns [`PluginError`] when plugin registration or module customization fails.
pub fn apply_observability_plugins(
    builder: ModuleDefinitionBuilder,
) -> Result<ModuleDefinitionBuilder, PluginError> {
    let mut registry = PluginRegistry::new();
    registry.register(ObservabilityPlugin)?;
    registry.apply(builder)
}
