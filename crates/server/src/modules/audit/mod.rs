//! Audit module definition for tamper-evident security logging.

pub mod controller;
pub mod dto;
pub mod entities;
pub mod repositories;
pub mod services;

use ironic::prelude::*;

pub use controller::AuditController;
pub use repositories::{AuditRepository, AuditStore, InMemoryAuditRepository, PgAuditRepository};
pub use services::AuditService;

#[derive(Module)]
#[module(
    imports = [crate::app::InfraModule],
    providers = [AuditService],
    controllers = [AuditController],
    exports = [AuditService],
)]
pub struct AuditModule;
