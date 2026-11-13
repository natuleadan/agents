//! DAG workflows module definition.

pub mod controller;
pub mod dto;
pub mod entities;
pub mod repositories;
pub mod services;

use ironic::prelude::*;

pub use controller::WorkflowsController;
pub use repositories::{
    InMemoryWorkflowRepository, PgWorkflowRepository, WorkflowRepository, WorkflowStore,
};
pub use services::{WorkflowService, evaluate_edge_condition, validate_dag_topological};

#[derive(Module)]
#[module(
    imports = [crate::app::InfraModule, crate::modules::audit::AuditModule],
    providers = [WorkflowService],
    controllers = [WorkflowsController],
    exports = [WorkflowService],
)]
pub struct WorkflowsModule;
