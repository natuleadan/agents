//! Infrastructure adapters for configuration, persistence, state, messaging, and admission control.

pub mod adapters;
pub mod admission;
pub mod bus;
pub mod cache;
pub mod config;
pub mod db;
pub mod doctor;
pub mod error;
pub mod events;
pub mod migrate;
pub mod observability;
pub mod state;

pub use observability::{
    DomainMetrics, LifecycleManager, LifecycleSnapshot, ObservabilityPlugin,
    PerRouteTimeoutInterceptor, ProviderHealthIndicator, TraceContextMiddleware, W3cTraceContext,
    apply_observability_plugins, inject_w3c_traceparent, slice_paginated,
};
pub mod stream;
pub mod webui;
pub mod workers;

pub use adapters::{
    BlobStoreClient, BlobStorePort, InfrastructureRegistry, LocalFsBlobStore, QstashMessageBus,
    UpstashBlobStore, UpstashVectorAdapter, VectorMatch, VectorStoreClient, VectorStorePort,
};
pub use admission::{AdmissionController, AdmissionPermit};
pub use bus::{InMemoryMessageBus, MessageBus, NatsMessageBus};
pub use cache::AppCache;
pub use config::{
    ConfigError, EnvironmentMode, InfrastructureMode, ServerConfig, generate_ephemeral_secret,
};
pub use db::{BusHealthIndicator, Database, PostgresHealthIndicator, StateHealthIndicator};
pub use doctor::{CheckStatus, DoctorItem, DoctorReport};
pub use error::{ErrorResponse, ServerError, validate_dto};
pub use events::{
    AppEventBus, AutocronTickEvent, SecurityAlertEvent, SessionCreatedEvent, SessionUpdatedEvent,
    SwarmPeerMessageEvent, TaskCompletedEvent,
};
pub use migrate::{
    ImportSummary, MigrationSnapshot, VerifyReport, export_snapshot, import_snapshot,
    validate_in_memory_audit_chain, verify_database,
};
pub use state::{
    InMemoryStateBackend, RateLimitDecision, StateBackend, ValkeyStateBackend, now_ms,
};
pub use stream::{
    AdmissionStore, BusClient, InMemoryStreamRepository, PgStreamRepository, StreamRepository,
    StreamStore, emit_stream_event,
};
