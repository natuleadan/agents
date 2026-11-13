//! Infrastructure adapters for configuration, persistence, state, messaging, and admission control.

pub mod admission;
pub mod bus;
pub mod config;
pub mod db;
pub mod error;
pub mod state;
pub mod stream;

pub use admission::{AdmissionController, AdmissionPermit};
pub use bus::{InMemoryMessageBus, MessageBus, NatsMessageBus};
pub use config::{ConfigError, EnvironmentMode, ServerConfig, generate_ephemeral_secret};
pub use db::{BusHealthIndicator, Database, PostgresHealthIndicator, StateHealthIndicator};
pub use error::{ServerError, validate_dto};
pub use state::{
    InMemoryStateBackend, RateLimitDecision, StateBackend, ValkeyStateBackend, now_ms,
};
pub use stream::{
    AdmissionStore, BusClient, InMemoryStreamRepository, PgStreamRepository, StreamRepository,
    StreamStore, emit_stream_event,
};
