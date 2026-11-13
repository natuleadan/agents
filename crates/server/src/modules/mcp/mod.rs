//! MCP host module definition.

pub mod controller;
pub mod dto;
pub mod services;

use ironic::prelude::*;

pub use controller::{McpController, McpHttpState, protected_mcp_router};
pub use services::McpHostService;

#[derive(Module)]
#[module(
    imports = [
        crate::app::InfraModule,
        crate::modules::audit::AuditModule,
        crate::modules::sessions::SessionsModule,
        crate::modules::tasks::TasksModule,
    ],
    providers = [McpHostService],
    controllers = [McpController],
    exports = [McpHostService],
)]
pub struct McpModule;
