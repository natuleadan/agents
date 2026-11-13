//! Sessions module definition.

pub mod controller;
pub mod dto;
pub mod entities;
pub mod repositories;
pub mod services;

use ironic::prelude::*;

pub use controller::SessionsController;
pub use repositories::{
    InMemorySessionRepository, PgSessionRepository, SessionRepository, SessionStore,
};
pub use services::SessionService;

#[derive(Module)]
#[module(
    imports = [crate::app::InfraModule, crate::modules::audit::AuditModule],
    providers = [SessionService],
    controllers = [SessionsController],
    exports = [SessionService],
)]
pub struct SessionsModule;
