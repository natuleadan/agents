//! Typed application domain event bus wrapping native Ironic events with dead-letter queue.

use std::sync::Arc;

use ironic::services::events::{EventBus, EventSubscription};
use serde::{Deserialize, Serialize};

/// High-priority security alert emitted when invariant violations or attack patterns occur.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct SecurityAlertEvent {
    pub code: String,
    pub message: String,
    pub actor_id: Option<String>,
    pub timestamp: u64,
}

/// Emitted when a new agent session is provisioned.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct SessionCreatedEvent {
    pub session_id: String,
    pub user_id: String,
    pub mode: String,
    pub created_at: u64,
}

/// Emitted when an agent session is modified or finished.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct SessionUpdatedEvent {
    pub session_id: String,
    pub user_id: String,
    pub status: String,
    pub updated_at: u64,
}

/// Emitted when an agent task reaches a terminal state.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct TaskCompletedEvent {
    pub task_id: String,
    pub session_id: String,
    pub status: String,
    pub completed_at: u64,
}

/// Emitted when an agent publishes a peer-to-peer message across the swarm blackboard.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct SwarmPeerMessageEvent {
    pub swarm_id: String,
    pub from_agent: String,
    pub to_agent: String,
    pub topic: String,
    pub sent_at: u64,
}

/// Emitted when an Autocron schedule tick fires and enqueues a workflow run.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct AutocronTickEvent {
    pub schedule_id: String,
    pub workflow_id: String,
    pub tick_at: u64,
}

/// Emitted when an external A2A message is accepted by the orchestrator postal gateway.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct A2aInboundTaskEvent {
    pub task_id: String,
    pub context_id: String,
    pub caller_org: Option<String>,
    pub caller_installation_id: Option<String>,
    pub received_at: u64,
}

/// Thread-safe injectable domain event bus wrapping native [`ironic::services::events::EventBus`].
#[derive(Clone, Default)]
pub struct AppEventBus {
    bus: Arc<EventBus>,
}

impl AppEventBus {
    /// Creates a new [`AppEventBus`].
    #[must_use]
    pub fn new() -> Self {
        Self {
            bus: Arc::new(EventBus::default()),
        }
    }

    /// Subscribes to events of type `E` with bounded channel capacity.
    pub async fn subscribe<E: Send + Sync + 'static>(
        &self,
        capacity: usize,
    ) -> EventSubscription<E> {
        self.bus.subscribe::<E>(capacity).await
    }

    /// Publishes a domain event across in-process subscribers.
    ///
    /// Returns the number of subscribers that accepted the event.
    pub async fn publish<E: Send + Sync + std::fmt::Debug + 'static>(&self, event: E) -> usize {
        self.bus.publish(event).await
    }
}
