//! Authentication and authorization module.

pub mod controller;
pub mod dto;
pub mod entities;
pub mod guards;
pub mod repositories;
pub mod services;

use ironic::prelude::*;

pub use controller::AuthController;
pub use repositories::{AuthRepository, AuthStore, InMemoryAuthRepository, PgAuthRepository};
pub use services::{AuthService, StateStore};

#[derive(Module)]
#[module(
    imports = [crate::app::InfraModule, crate::modules::audit::AuditModule],
    providers = [AuthService],
    controllers = [AuthController],
    exports = [AuthService],
)]
pub struct AuthModule;
