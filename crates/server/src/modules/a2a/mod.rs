//! Agent-to-Agent (A2A) v1 protocol module built natively on `a2a-lf` and `IdentityService`.

pub mod controller;
pub mod repositories;
pub mod services;

use ironic::prelude::*;

pub use controller::{A2aDiscoveryController, A2aOutboundController, A2aRpcController};
pub use repositories::{A2aRepository, A2aTaskStore, InMemoryA2aRepository, PgA2aRepository};
pub use services::{A2aService, OutboundA2aResponse, OutboundA2aSendDto};

#[derive(Module)]
#[module(
    imports = [crate::app::InfraModule, crate::modules::identity::IdentityModule],
    providers = [A2aService],
    controllers = [A2aDiscoveryController, A2aRpcController, A2aOutboundController],
    exports = [A2aService],
)]
pub struct A2aModule;
