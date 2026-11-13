//! Private-VLAN Boxer discovery and distributed preview route module.

pub mod controller;
pub mod dto;
pub mod entities;
pub mod services;

use ironic::prelude::*;

pub use controller::{NodeHttpState, NodesController, traefik_lazy_router};
pub use services::{NodeRegistryService, NodeRegistryStore};

/// Registers the node registry service and HTTP controller.
#[derive(Module)]
#[module(
    imports = [crate::app::InfraModule, crate::modules::audit::AuditModule],
    providers = [NodeRegistryService],
    controllers = [NodesController],
    exports = [NodeRegistryService],
)]
pub struct NodesModule;
