//! Orchestrator server library exposing the Ironic modular monolith and daemon wiring.

#![forbid(unsafe_code)]

pub mod app;
pub mod infra;
pub mod modules;
pub mod openapi;

pub use app::{AppModule, InfraModule, InfrastructureHandles, build_application};
pub use infra::{
    CheckStatus, ConfigError, DoctorReport, EnvironmentMode, ErrorResponse, InfrastructureMode,
    MigrationSnapshot, ServerConfig, ServerError, export_snapshot, import_snapshot,
    verify_database,
};
pub use openapi::configure_openapi;
