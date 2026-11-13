//! Decoupled host lifecycle hooks (`prepare_request`, `prepare_next_turn`, `finish_turn`),
//! session-level exponential backoff retry policy, and CodeMode structured JSON contracts.

use std::time::Duration;

use agents_protocol::{FollowUpMessage, InteractionMode, TurnDecision};
use serde_json::Value;

use crate::{
    error::BoxerError,
    leases::BoxFuture,
    runtime::{
        providers::{ModelTurnRequest, ModelTurnResponse},
        session_tree::SessionTree,
    },
};

/// Exponential backoff configuration for transient model provider errors.
#[derive(Debug, Clone, Copy)]
pub struct RetryPolicy {
    pub max_retries: u32,
    pub base_delay_ms: u64,
    pub max_delay_ms: u64,
}

impl Default for RetryPolicy {
    fn default() -> Self {
        Self {
            max_retries: 2,
            base_delay_ms: 50,
            max_delay_ms: 2_000,
        }
    }
}

impl RetryPolicy {
    #[must_use]
    pub fn backoff_for_attempt(&self, attempt: u32) -> Duration {
        let factor = 1u64.checked_shl(attempt.min(10)).unwrap_or(1024);
        let ms = self
            .base_delay_ms
            .saturating_mul(factor)
            .min(self.max_delay_ms);
        Duration::from_millis(ms)
    }
}

/// Optional CodeMode structured JSON schema guard verified on final assistant output.
#[derive(Debug, Clone)]
pub struct OutputSchemaContract {
    pub required_fields: Vec<String>,
}

impl OutputSchemaContract {
    /// Parses `text` as JSON object and checks every required top-level key is present.
    ///
    /// # Errors
    ///
    /// Returns [`BoxerError::ExecutionFailed`] if output violates structured JSON rules.
    pub fn validate(&self, text: &str) -> Result<Value, BoxerError> {
        let parsed: Value = serde_json::from_str(text.trim()).map_err(|err| {
            BoxerError::ExecutionFailed(format!("CodeMode structured JSON expected: {err}"))
        })?;
        let Some(obj) = parsed.as_object() else {
            return Err(BoxerError::ExecutionFailed(
                "CodeMode output must be a JSON object".into(),
            ));
        };
        for key in &self.required_fields {
            if !obj.contains_key(key) {
                return Err(BoxerError::ExecutionFailed(format!(
                    "CodeMode output missing required field `{key}`"
                )));
            }
        }
        Ok(parsed)
    }
}

/// Host lifecycle hooks decoupling model loop execution from policy/UI/orchestration concerns.
pub trait LoopHooks: Send + Sync + 'static {
    fn prepare_request<'a>(
        &'a self,
        _tree: &'a SessionTree,
        req: ModelTurnRequest,
        _mode: InteractionMode,
    ) -> BoxFuture<'a, Result<ModelTurnRequest, BoxerError>> {
        Box::pin(async move { Ok(req) })
    }

    fn prepare_next_turn<'a>(
        &'a self,
        _turn_index: u32,
        _tree: &'a SessionTree,
        pending_follow_up: Option<&'a FollowUpMessage>,
    ) -> BoxFuture<'a, TurnDecision> {
        Box::pin(async move {
            if pending_follow_up.is_some() {
                TurnDecision::Continue
            } else {
                TurnDecision::End
            }
        })
    }

    fn finish_turn<'a>(
        &'a self,
        _turn_index: u32,
        _response: &'a ModelTurnResponse,
    ) -> BoxFuture<'a, Result<(), BoxerError>> {
        Box::pin(async { Ok(()) })
    }
}

/// Default no-op host hook policy continuing on queued follow-ups and ending otherwise.
#[derive(Debug, Default, Clone, Copy)]
pub struct DefaultLoopHooks;

impl LoopHooks for DefaultLoopHooks {}
