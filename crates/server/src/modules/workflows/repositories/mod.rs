//! Persistence repository for DAG workflows, dynamic slugs, and runs.

pub mod memory;
pub mod pg;

use std::sync::Arc;

use crate::{
    infra::{error::ServerError, state::BoxFuture},
    modules::workflows::entities::{RenderedSlugEntity, WorkflowEntity, WorkflowRunEntity},
};

pub use memory::InMemoryWorkflowRepository;
pub use pg::PgWorkflowRepository;

/// Concrete DI wrapper around [`WorkflowRepository`].
#[derive(Clone)]
pub struct WorkflowStore(pub Arc<dyn WorkflowRepository>);

impl Default for WorkflowStore {
    fn default() -> Self {
        Self(InMemoryWorkflowRepository::new())
    }
}

/// Persistence contract for workflows, dynamic slugs, and checkpointed execution runs.
pub trait WorkflowRepository: Send + Sync + 'static {
    fn create_workflow<'a>(
        &'a self,
        wf: WorkflowEntity,
    ) -> BoxFuture<'a, Result<WorkflowEntity, ServerError>>;

    fn find_workflow<'a>(
        &'a self,
        workflow_id: &'a str,
    ) -> BoxFuture<'a, Result<Option<WorkflowEntity>, ServerError>>;

    fn list_workflows_for_owner<'a>(
        &'a self,
        owner_id: &'a str,
    ) -> BoxFuture<'a, Result<Vec<WorkflowEntity>, ServerError>>;

    fn find_workflow_by_webhook_slug<'a>(
        &'a self,
        slug: &'a str,
    ) -> BoxFuture<'a, Result<Option<WorkflowEntity>, ServerError>>;

    fn find_workflow_by_render_slug<'a>(
        &'a self,
        slug: &'a str,
    ) -> BoxFuture<'a, Result<Option<WorkflowEntity>, ServerError>>;

    fn approve_workflow<'a>(
        &'a self,
        workflow_id: &'a str,
        approver_id: &'a str,
        approved_at: u64,
    ) -> BoxFuture<'a, Result<WorkflowEntity, ServerError>>;

    fn upsert_rendered_slug<'a>(
        &'a self,
        entity: RenderedSlugEntity,
    ) -> BoxFuture<'a, Result<RenderedSlugEntity, ServerError>>;

    fn find_rendered_slug<'a>(
        &'a self,
        slug: &'a str,
    ) -> BoxFuture<'a, Result<Option<RenderedSlugEntity>, ServerError>>;

    fn upsert_run<'a>(
        &'a self,
        run: WorkflowRunEntity,
    ) -> BoxFuture<'a, Result<WorkflowRunEntity, ServerError>>;

    fn find_run<'a>(
        &'a self,
        run_id: &'a str,
    ) -> BoxFuture<'a, Result<Option<WorkflowRunEntity>, ServerError>>;

    fn list_stale_running_runs<'a>(
        &'a self,
        current_boot_id: &'a str,
    ) -> BoxFuture<'a, Result<Vec<WorkflowRunEntity>, ServerError>>;
}
