//! Gateway module definition.

pub mod controller;
pub mod dto;
pub mod services;

use ironic::prelude::*;

pub use controller::{
    GatewayController, GatewayHttpState, gateway_lazy_router, gateway_stream_router,
};
pub use services::GatewayService;

#[derive(Module)]
#[module(
    imports = [
        crate::app::InfraModule,
        crate::modules::sessions::SessionsModule,
        crate::modules::tasks::TasksModule,
    ],
    providers = [GatewayService],
    controllers = [GatewayController],
    exports = [GatewayService],
)]
pub struct GatewayModule;
