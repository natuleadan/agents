//! Installation-wide identity management and indefinite agent interaction ledger module.

pub mod controller;
pub mod entities;
pub mod repositories;
pub mod services;

use ironic::prelude::*;

pub use controller::IdentityController;
pub use repositories::{
    IdentityRepository, IdentityStore, InMemoryIdentityRepository, PgIdentityRepository,
};
pub use services::IdentityService;

#[derive(Module)]
#[module(
    imports = [crate::app::InfraModule],
    providers = [IdentityService],
    controllers = [IdentityController],
    exports = [IdentityService],
)]
pub struct IdentityModule;
