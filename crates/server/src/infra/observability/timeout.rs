//! Per-route timeout interceptor returning standard HTTP 408 Request Timeout.

use std::time::Duration;

use ironic::{HttpError, HttpStatus, Interceptor, InterceptorNext, PipelineFuture, RequestContext};

use super::metrics::DomainMetrics;

/// Per-route Ironic [`Interceptor`] bounding execution duration and returning HTTP `408`.
#[derive(Clone, Copy, Debug)]
pub struct PerRouteTimeoutInterceptor {
    timeout: Duration,
}

impl PerRouteTimeoutInterceptor {
    /// Creates a timeout interceptor with an arbitrary [`Duration`].
    #[must_use]
    pub const fn new(timeout: Duration) -> Self {
        Self { timeout }
    }

    /// Creates a timeout interceptor configured in whole seconds.
    #[must_use]
    pub const fn secs(seconds: u64) -> Self {
        Self {
            timeout: Duration::from_secs(seconds),
        }
    }

    /// Creates a timeout interceptor configured in milliseconds.
    #[must_use]
    pub const fn millis(ms: u64) -> Self {
        Self {
            timeout: Duration::from_millis(ms),
        }
    }
}

impl Interceptor for PerRouteTimeoutInterceptor {
    fn intercept<'a>(
        &'a self,
        context: &'a mut RequestContext,
        next: InterceptorNext<'a>,
    ) -> PipelineFuture<'a> {
        let limit = self.timeout;
        Box::pin(async move {
            match tokio::time::timeout(limit, next.run(context)).await {
                Ok(inner) => inner,
                Err(_elapsed) => {
                    DomainMetrics::global().timeouts_total.inc();
                    tracing::warn!(
                        timeout_ms = limit.as_millis(),
                        "route exceeded per-route timeout"
                    );
                    Err(HttpError::new(
                        HttpStatus::REQUEST_TIMEOUT,
                        "ROUTE_TIMEOUT",
                        format!(
                            "route exceeded configured timeout of {}ms",
                            limit.as_millis()
                        ),
                    ))
                }
            }
        })
    }
}
