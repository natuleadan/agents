//! Application Performance Monitoring (APM) and OTLP exporter integration.

use std::time::Duration;

use serde::{Deserialize, Serialize};

/// Configuration for Application Performance Monitoring and OpenTelemetry collector export.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ApmConfig {
    /// Service name identifying the server in APM dashboards (Jaeger, Tempo, SigNoz).
    pub service_name: String,
    /// OTLP gRPC/HTTP endpoint (e.g. `http://127.0.0.1:4317`), or `None` for local-only tracing.
    pub otlp_endpoint: Option<String>,
    /// Trace sampling ratio between 0.0 (none) and 1.0 (all requests).
    pub sample_rate: f64,
    /// Batch flush interval for OTLP span exports in milliseconds.
    pub batch_interval_ms: u64,
    /// Whether W3C `traceparent` propagation across downstream hops is enabled.
    pub propagate_context: bool,
}

impl Default for ApmConfig {
    fn default() -> Self {
        Self {
            service_name: "agents-server".to_owned(),
            otlp_endpoint: None,
            sample_rate: 1.0,
            batch_interval_ms: 2000,
            propagate_context: true,
        }
    }
}

impl ApmConfig {
    /// Loads APM configuration from environment variables (`AGENTS_OTEL_ENDPOINT` / `OTLP_ENDPOINT`).
    #[must_use]
    pub fn from_env() -> Self {
        let endpoint = std::env::var("AGENTS_OTEL_ENDPOINT")
            .or_else(|_| std::env::var("OTLP_ENDPOINT"))
            .ok()
            .map(|s| s.trim().to_owned())
            .filter(|s| !s.is_empty());

        let sample_rate = std::env::var("AGENTS_OTEL_SAMPLE_RATE")
            .or_else(|_| std::env::var("SAMPLE_RATE"))
            .ok()
            .and_then(|s| s.parse::<f64>().ok())
            .unwrap_or(1.0)
            .clamp(0.0, 1.0);

        let batch_interval_ms = std::env::var("AGENTS_OTEL_BATCH_MS")
            .ok()
            .and_then(|s| s.parse::<u64>().ok())
            .unwrap_or(2000);

        Self {
            service_name: "agents-server".to_owned(),
            otlp_endpoint: endpoint,
            sample_rate,
            batch_interval_ms,
            propagate_context: true,
        }
    }

    /// Returns the batch export interval as a [`Duration`].
    #[must_use]
    pub const fn batch_interval(&self) -> Duration {
        Duration::from_millis(self.batch_interval_ms)
    }

    /// Whether active network OTLP trace export is configured.
    #[must_use]
    pub const fn is_export_enabled(&self) -> bool {
        self.otlp_endpoint.is_some()
    }
}
