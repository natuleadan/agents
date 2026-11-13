//! Tasks module definition.

pub mod controller;
pub mod dto;
pub mod entities;
pub mod repositories;
pub mod services;

use ironic::prelude::*;

pub use controller::TasksController;
pub use repositories::{InMemoryTaskRepository, PgTaskRepository, TaskRepository, TaskStore};
pub use services::TaskService;

#[derive(Module)]
#[module(
    imports = [
        crate::app::InfraModule,
        crate::modules::audit::AuditModule,
        crate::modules::sessions::SessionsModule,
    ],
    providers = [TaskService],
    controllers = [TasksController],
    exports = [TaskService],
)]
pub struct TasksModule;
