//! Observability infrastructure: metrics, distributed tracing, APM, logging, lifecycle, health, timeouts, plugins, and pagination.

pub mod apm;
pub mod health;
pub mod lifecycle;
pub mod logging;
pub mod metrics;
pub mod pagination;
pub mod plugin;
pub mod timeout;
pub mod tracing;

pub use apm::ApmConfig;
pub use health::ProviderHealthIndicator;
pub use lifecycle::{LifecycleManager, LifecycleSnapshot};
pub use logging::LoggingConfig;
pub use metrics::DomainMetrics;
pub use pagination::slice_paginated;
pub use plugin::{ObservabilityPlugin, apply_observability_plugins};
pub use timeout::PerRouteTimeoutInterceptor;
pub use tracing::{TraceContextMiddleware, W3cTraceContext, inject_w3c_traceparent};
