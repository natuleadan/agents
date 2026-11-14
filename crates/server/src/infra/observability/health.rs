//! Dependency injection container health indicator (`Container::health`).

use std::{future::Future, pin::Pin};

use ironic::{Container, HealthIndicator, HealthStatus};

/// Ironic DI [`HealthIndicator`] inspecting `Container::health()` constructor and error counts.
#[derive(Clone)]
pub struct ProviderHealthIndicator {
    container: Container,
}

impl ProviderHealthIndicator {
    /// Creates a new [`ProviderHealthIndicator`] bound to the compiled DI [`Container`].
    #[must_use]
    pub const fn new(container: Container) -> Self {
        Self { container }
    }
}

impl HealthIndicator for ProviderHealthIndicator {
    fn name(&self) -> &'static str {
        "providers"
    }

    #[allow(deprecated)]
    fn check(&self) -> Pin<Box<dyn Future<Output = HealthStatus> + Send + '_>> {
        self.check_readiness()
    }

    fn check_readiness(&self) -> Pin<Box<dyn Future<Output = HealthStatus> + Send + '_>> {
        let summary = self.container.health();
        Box::pin(async move {
            if summary.total_providers == 0 {
                return HealthStatus::Unhealthy {
                    error: "DI container has zero registered providers".to_owned(),
                };
            }
            let failing: Vec<String> = summary
                .providers
                .iter()
                .filter(|(_, h)| h.error_count > 0)
                .map(|(k, h)| {
                    let msg = h.last_error.as_deref().unwrap_or("constructor error");
                    format!("{k}: {} error(s) ({msg})", h.error_count)
                })
                .collect();
            if failing.is_empty() {
                HealthStatus::Ok
            } else {
                HealthStatus::Degraded {
                    message: Some(failing.join("; ")),
                }
            }
        })
    }
}
