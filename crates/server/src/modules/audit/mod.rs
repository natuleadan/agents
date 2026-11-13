//! Audit module definition for tamper-evident security logging.

pub mod controller;
pub mod dto;
pub mod entities;
pub mod repositories;
pub mod services;

use ironic::prelude::*;

pub use controller::AuditController;
pub use repositories::{
    AuditRepository, AuditStore, GENESIS_AUDIT_HASH, InMemoryAuditRepository, PgAuditRepository,
};
pub use services::{AuditService, compute_entry_hash};

#[derive(Module)]
#[module(
    imports = [crate::app::InfraModule],
    providers = [AuditService],
    controllers = [AuditController],
    exports = [AuditService],
)]
pub struct AuditModule;
