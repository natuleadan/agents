//! Distributed tracing and W3C `traceparent` context propagation.

use std::time::Instant;

use ironic::{
    HeaderMap, HeaderName, HeaderValue, Middleware, MiddlewareNext, PipelineFuture, RequestContext,
};
use uuid::Uuid;

use super::metrics::DomainMetrics;

/// Parsed or freshly minted W3C Trace Context (`00-{trace_id}-{span_id}-{flags}`).
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct W3cTraceContext {
    /// 32-character lowercase hex trace identifier.
    pub trace_id: String,
    /// 16-character lowercase hex span identifier for the current hop.
    pub span_id: String,
    /// Complete formatted `traceparent` header value.
    pub traceparent: String,
}

impl W3cTraceContext {
    /// Extracts a valid W3C `traceparent` header (preserving `trace_id`) or mints a new trace.
    #[must_use]
    pub fn from_headers_or_new(headers: &HeaderMap) -> Self {
        if let Some(trace_id) = headers
            .get("traceparent")
            .and_then(|value| value.to_str().ok())
            .and_then(parse_w3c_trace_id)
        {
            return Self::with_trace_id(trace_id);
        }
        Self::new()
    }

    /// Generates a brand new root [`W3cTraceContext`] backed by UUIDv4 randomness.
    #[must_use]
    pub fn new() -> Self {
        let trace_id = format!("{:032x}", Uuid::new_v4().as_u128());
        Self::with_trace_id(trace_id)
    }

    fn with_trace_id(trace_id: String) -> Self {
        let span_u64 = (Uuid::new_v4().as_u128() >> 64) as u64;
        let span_val = if span_u64 == 0 { 1 } else { span_u64 };
        let span_id = format!("{span_val:016x}");
        let traceparent = format!("00-{trace_id}-{span_id}-01");
        Self {
            trace_id,
            span_id,
            traceparent,
        }
    }
}

impl Default for W3cTraceContext {
    fn default() -> Self {
        Self::new()
    }
}

fn parse_w3c_trace_id(raw: &str) -> Option<String> {
    let mut parts = raw.trim().split('-');
    let version = parts.next()?;
    let trace_id = parts.next()?;
    let span_id = parts.next()?;
    let flags = parts.next()?;
    if parts.next().is_some() {
        return None;
    }
    let valid = version.len() == 2
        && version != "ff"
        && version.bytes().all(|b| b.is_ascii_hexdigit())
        && trace_id.len() == 32
        && trace_id.bytes().all(|b| b.is_ascii_hexdigit())
        && trace_id.bytes().any(|b| b != b'0')
        && span_id.len() == 16
        && span_id.bytes().all(|b| b.is_ascii_hexdigit())
        && span_id.bytes().any(|b| b != b'0')
        && flags.len() == 2
        && flags.bytes().all(|b| b.is_ascii_hexdigit());
    if valid {
        Some(trace_id.to_ascii_lowercase())
    } else {
        None
    }
}

/// Returns `(header_name, header_value)` for propagating W3C `traceparent` to downstream hops.
#[must_use]
pub fn inject_w3c_traceparent(trace: Option<&W3cTraceContext>) -> (&'static str, String) {
    let value = match trace {
        Some(existing) => existing.traceparent.clone(),
        None => W3cTraceContext::new().traceparent,
    };
    ("traceparent", value)
}

/// Global Ironic [`Middleware`] propagating W3C `traceparent` + `x-trace-id` headers
/// and recording request telemetry into [`DomainMetrics`].
#[derive(Clone, Default)]
pub struct TraceContextMiddleware;

impl TraceContextMiddleware {
    /// Creates a new [`TraceContextMiddleware`].
    #[must_use]
    pub const fn new() -> Self {
        Self
    }
}

impl Middleware for TraceContextMiddleware {
    fn handle<'a>(
        &'a self,
        context: &'a mut RequestContext,
        next: MiddlewareNext<'a>,
    ) -> PipelineFuture<'a> {
        let trace = W3cTraceContext::from_headers_or_new(context.request().headers());
        context.insert_extension(trace.clone());
        let started = Instant::now();
        Box::pin(async move {
            let result = next.run(context).await;
            let elapsed_secs = started.elapsed().as_secs_f64();
            let metrics = DomainMetrics::global();
            metrics.requests_handled.inc();
            metrics.request_latency_ms.record(elapsed_secs);

            match result {
                Ok(mut response) => {
                    if let Ok(tp) = HeaderValue::from_str(&trace.traceparent) {
                        response
                            .headers_mut()
                            .insert(HeaderName::from_static("traceparent"), tp);
                    }
                    if let Ok(tid) = HeaderValue::from_str(&trace.trace_id) {
                        response
                            .headers_mut()
                            .insert(HeaderName::from_static("x-trace-id"), tid);
                    }
                    Ok(response)
                }
                Err(err) => Err(err),
            }
        })
    }
}
