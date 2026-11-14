//! Domain and RED metrics backed by Ironic's global [`MetricsRegistry`].

use std::sync::{Arc, OnceLock};

use ironic::metrics::{Counter, Gauge, Histogram, MetricsRegistry};

/// Shared application domain metrics registered in Ironic's global [`MetricsRegistry`].
#[derive(Clone)]
pub struct DomainMetrics {
    /// Total HTTP requests observed across the TraceContext middleware.
    pub requests_handled: Arc<Counter>,
    /// Total controller executions rejected by timeout interceptors.
    pub timeouts_total: Arc<Counter>,
    /// Total requests served by native paginated endpoints.
    pub paginated_queries_total: Arc<Counter>,
    /// Total lifecycle phase callbacks fired on [`super::LifecycleManager`].
    pub lifecycle_transitions_total: Arc<Counter>,
    /// Currently active in-flight workflow dispatches.
    pub workflow_runs_inflight: Arc<Gauge>,
    /// End-to-end request duration in milliseconds observed at the middleware boundary.
    pub request_latency_ms: Arc<Histogram>,
}

static DOMAIN_METRICS: OnceLock<DomainMetrics> = OnceLock::new();

impl DomainMetrics {
    /// Returns the process-wide singleton [`DomainMetrics`] handle.
    #[must_use]
    pub fn global() -> &'static Self {
        DOMAIN_METRICS.get_or_init(|| Self {
            requests_handled: MetricsRegistry.counter(
                "agents_domain_requests_total",
                "Total domain HTTP requests handled by agents-server",
            ),
            timeouts_total: MetricsRegistry.counter(
                "agents_route_timeouts_total",
                "Total controller route invocations terminated by per-route timeout",
            ),
            paginated_queries_total: MetricsRegistry.counter(
                "agents_paginated_queries_total",
                "Total paginated list queries executed with ironic::Pagination",
            ),
            lifecycle_transitions_total: MetricsRegistry.counter(
                "agents_lifecycle_transitions_total",
                "Total Ironic module/application lifecycle transitions executed",
            ),
            workflow_runs_inflight: MetricsRegistry.gauge(
                "agents_workflow_runs_inflight",
                "Current number of in-flight workflow executions",
            ),
            request_latency_ms: MetricsRegistry.histogram(
                "agents_domain_request_duration_ms",
                "Middleware-observed HTTP request latency in milliseconds",
            ),
        })
    }
}
