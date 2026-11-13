//! Autocron scheduling and crash-resume workflow dispatch module.

pub mod controller;
pub mod dto;
pub mod entities;
pub mod repositories;
pub mod services;

use ironic::prelude::*;

pub use controller::AutocronController;
pub use repositories::{
    AutocronRepository, AutocronStore, InMemoryAutocronRepository, PgAutocronRepository,
};
pub use services::{AutocronService, cron};

#[derive(Module)]
#[module(
    imports = [
        crate::app::InfraModule,
        crate::modules::audit::AuditModule,
        crate::modules::workflows::WorkflowsModule,
    ],
    providers = [AutocronService],
    controllers = [AutocronController],
    exports = [AutocronService],
)]
pub struct AutocronModule;
