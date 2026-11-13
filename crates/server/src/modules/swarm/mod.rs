//! Multi-agent swarm coordination module definition.

pub mod controller;
pub mod dto;
pub mod entities;
pub mod repositories;
pub mod services;

use ironic::prelude::*;

pub use controller::SwarmController;
pub use repositories::{InMemorySwarmRepository, PgSwarmRepository, SwarmRepository, SwarmStore};
pub use services::SwarmService;

#[derive(Module)]
#[module(
    imports = [
        crate::app::InfraModule,
        crate::modules::audit::AuditModule,
        crate::modules::sessions::SessionsModule,
    ],
    providers = [SwarmService],
    controllers = [SwarmController],
    exports = [SwarmService],
)]
pub struct SwarmModule;
