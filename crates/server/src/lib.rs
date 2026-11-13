//! Orchestrator server library exposing the Ironic modular monolith and daemon wiring.

#![forbid(unsafe_code)]

pub mod app;
pub mod infra;
pub mod modules;

pub use app::{AppModule, InfraModule, InfrastructureHandles, build_application};
pub use infra::{ConfigError, EnvironmentMode, ServerConfig, ServerError};
