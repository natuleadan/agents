//! Security lifecycle hooks (`OnGuardDenied`, `OnError`) and circuit-breaker HTTP middleware.

use std::sync::Arc;

use ironic::{
    HttpError, HttpStatus, LifecycleFuture, Middleware, MiddlewareNext, OnError, OnGuardDenied,
    PipelineFuture, RequestContext,
};

use crate::{
    infra::{
        events::{AppEventBus, SecurityAlertEvent},
        state::now_ms,
    },
    modules::auth::{entities::AuthenticatedActor, guards::AgentCircuitBreaker},
};

fn now_i64() -> i64 {
    i64::try_from(now_ms()).unwrap_or(i64::MAX)
}

/// Resolves an agent identifier for circuit-breaking and anomaly alerts from
/// explicit `X-Agent-Id` / `X-Box-Id` headers or the authenticated [`AuthenticatedActor`].
fn resolve_agent_key(context: &RequestContext) -> Option<String> {
    if let Some(id) = context
        .request()
        .headers()
        .get("x-agent-id")
        .or_else(|| context.request().headers().get("x-box-id"))
        .and_then(|v| v.to_str().ok())
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        return Some(id.to_owned());
    }
    context
        .extension::<AuthenticatedActor>()
        .map(|actor| actor.user_id.clone())
}

/// Ironic lifecycle hook publishing structured [`SecurityAlertEvent`]s on guard denials
/// and unexpected application errors.
#[derive(Clone, Default)]
pub struct SecurityLifecycleHook {
    events: Arc<AppEventBus>,
}

impl SecurityLifecycleHook {
    /// Creates a new [`SecurityLifecycleHook`] bound to the cluster [`AppEventBus`].
    #[must_use]
    pub const fn new(events: Arc<AppEventBus>) -> Self {
        Self { events }
    }

    /// Returns the shared [`AppEventBus`] handle.
    #[must_use]
    pub fn events(&self) -> &Arc<AppEventBus> {
        &self.events
    }
}

impl OnGuardDenied for SecurityLifecycleHook {
    fn on_guard_denied(&self, guard_name: &str) -> LifecycleFuture<'_> {
        let events = Arc::clone(&self.events);
        let guard = guard_name.to_owned();
        Box::pin(async move {
            tracing::warn!(guard = %guard, "security lifecycle: guard denied request");
            let _ = events
                .publish(SecurityAlertEvent {
                    code: "SECURITY_GUARD_DENIED".to_owned(),
                    message: format!("Access blocked by guard or authorization check: {guard}"),
                    actor_id: None,
                    timestamp: now_ms(),
                })
                .await;
            Ok(())
        })
    }
}

impl OnError for SecurityLifecycleHook {
    fn on_error(&self, error_code: &str, error_message: &str) -> LifecycleFuture<'_> {
        let events = Arc::clone(&self.events);
        let code = error_code.to_owned();
        let message = error_message.to_owned();
        Box::pin(async move {
            tracing::warn!(code = %code, "security lifecycle: runtime error captured");
            let _ = events
                .publish(SecurityAlertEvent {
                    code,
                    message,
                    actor_id: None,
                    timestamp: now_ms(),
                })
                .await;
            Ok(())
        })
    }
}

/// Global Ironic [`Middleware`] enforcing [`AgentCircuitBreaker`] lockouts and bridging
/// guard/error events into [`SecurityLifecycleHook`].
pub struct AgentCircuitBreakerMiddleware {
    breaker: AgentCircuitBreaker,
    hook: SecurityLifecycleHook,
}

impl AgentCircuitBreakerMiddleware {
    /// Creates a middleware instance backed by shared circuit-breaker state and lifecycle hooks.
    #[must_use]
    pub const fn new(breaker: AgentCircuitBreaker, hook: SecurityLifecycleHook) -> Self {
        Self { breaker, hook }
    }
}

impl Middleware for AgentCircuitBreakerMiddleware {
    fn handle<'a>(
        &'a self,
        context: &'a mut RequestContext,
        next: MiddlewareNext<'a>,
    ) -> PipelineFuture<'a> {
        Box::pin(async move {
            let now = now_i64();
            let pre_key = resolve_agent_key(context);
            if let Some(ref key) = pre_key
                && self.breaker.is_tripped(key, now).await
            {
                let _ = self
                    .hook
                    .events()
                    .publish(SecurityAlertEvent {
                        code: "AGENT_CIRCUIT_OPEN".to_owned(),
                        message: format!("Blocked request from circuit-broken agent `{key}`"),
                        actor_id: Some(key.clone()),
                        timestamp: now_ms(),
                    })
                    .await;
                return Err(HttpError::new(
                    HttpStatus::TOO_MANY_REQUESTS,
                    "AGENT_CIRCUIT_OPEN",
                    "Agent circuit breaker tripped due to repeated violations",
                ));
            }

            let result = next.run(context).await;
            if let Err(ref err) = result {
                let status = err.status();
                let post_key = resolve_agent_key(context).or(pre_key);
                if status == HttpStatus::TOO_MANY_REQUESTS {
                    if let Some(ref key) = post_key {
                        let tripped = self.breaker.record_violation(key, now_i64()).await;
                        let _ = self
                            .hook
                            .events()
                            .publish(SecurityAlertEvent {
                                code: if tripped {
                                    "AGENT_CIRCUIT_TRIPPED".to_owned()
                                } else {
                                    "AGENT_RATE_VIOLATION".to_owned()
                                },
                                message: format!(
                                    "Rate violation recorded for agent `{key}` (tripped={tripped})"
                                ),
                                actor_id: Some(key.clone()),
                                timestamp: now_ms(),
                            })
                            .await;
                    }
                } else if status == HttpStatus::UNAUTHORIZED || status == HttpStatus::FORBIDDEN {
                    let _ = self.hook.on_guard_denied(err.code()).await;
                } else if status.is_server_error() {
                    let _ = self.hook.on_error(err.code(), err.message()).await;
                }
            }
            result
        })
    }
}
