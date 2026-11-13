//! Deterministic in-memory workflow and dynamic slug repository.

use std::{collections::HashMap, sync::Arc};

use tokio::sync::Mutex;

use crate::{
    infra::{error::ServerError, state::BoxFuture},
    modules::workflows::entities::{
        RenderedSlugEntity, WorkflowEntity, WorkflowRunEntity, WorkflowRunStatus,
    },
    modules::workflows::repositories::WorkflowRepository,
};

/// Deterministic in-memory workflow and run checkpoint repository.
#[derive(Clone, Default)]
pub struct InMemoryWorkflowRepository {
    workflows: Arc<Mutex<HashMap<String, WorkflowEntity>>>,
    rendered_slugs: Arc<Mutex<HashMap<String, RenderedSlugEntity>>>,
    runs: Arc<Mutex<HashMap<String, WorkflowRunEntity>>>,
}

impl InMemoryWorkflowRepository {
    #[must_use]
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }
}

impl WorkflowRepository for InMemoryWorkflowRepository {
    fn create_workflow<'a>(
        &'a self,
        wf: WorkflowEntity,
    ) -> BoxFuture<'a, Result<WorkflowEntity, ServerError>> {
        Box::pin(async move {
            self.workflows
                .lock()
                .await
                .insert(wf.workflow_id.clone(), wf.clone());
            Ok(wf)
        })
    }

    fn find_workflow<'a>(
        &'a self,
        workflow_id: &'a str,
    ) -> BoxFuture<'a, Result<Option<WorkflowEntity>, ServerError>> {
        Box::pin(async move { Ok(self.workflows.lock().await.get(workflow_id).cloned()) })
    }

    fn list_workflows_for_owner<'a>(
        &'a self,
        owner_id: &'a str,
    ) -> BoxFuture<'a, Result<Vec<WorkflowEntity>, ServerError>> {
        Box::pin(async move {
            Ok(self
                .workflows
                .lock()
                .await
                .values()
                .filter(|w| w.owner_id == owner_id)
                .cloned()
                .collect())
        })
    }

    fn find_workflow_by_webhook_slug<'a>(
        &'a self,
        slug: &'a str,
    ) -> BoxFuture<'a, Result<Option<WorkflowEntity>, ServerError>> {
        Box::pin(async move {
            Ok(self
                .workflows
                .lock()
                .await
                .values()
                .find(|w| w.webhook_slug.as_deref() == Some(slug))
                .cloned())
        })
    }

    fn find_workflow_by_render_slug<'a>(
        &'a self,
        slug: &'a str,
    ) -> BoxFuture<'a, Result<Option<WorkflowEntity>, ServerError>> {
        Box::pin(async move {
            Ok(self
                .workflows
                .lock()
                .await
                .values()
                .find(|w| w.render_slug.as_deref() == Some(slug))
                .cloned())
        })
    }

    fn approve_workflow<'a>(
        &'a self,
        workflow_id: &'a str,
        approver_id: &'a str,
        approved_at: u64,
    ) -> BoxFuture<'a, Result<WorkflowEntity, ServerError>> {
        Box::pin(async move {
            let mut lock = self.workflows.lock().await;
            let wf = lock.get_mut(workflow_id).ok_or_else(|| {
                ServerError::NotFound(format!("workflow `{workflow_id}` not found"))
            })?;
            wf.approved = true;
            wf.approved_by = Some(approver_id.to_owned());
            wf.approved_at = Some(approved_at);
            wf.updated_at = approved_at;

            // Also mark any associated rendered slug approved
            if let Some(render_slug) = &wf.render_slug {
                let mut rendered_lock = self.rendered_slugs.lock().await;
                if let Some(slug_entity) = rendered_lock.get_mut(render_slug) {
                    slug_entity.approved = true;
                    slug_entity.updated_at = approved_at;
                }
            }

            Ok(wf.clone())
        })
    }

    fn upsert_rendered_slug<'a>(
        &'a self,
        entity: RenderedSlugEntity,
    ) -> BoxFuture<'a, Result<RenderedSlugEntity, ServerError>> {
        Box::pin(async move {
            self.rendered_slugs
                .lock()
                .await
                .insert(entity.slug.clone(), entity.clone());
            Ok(entity)
        })
    }

    fn find_rendered_slug<'a>(
        &'a self,
        slug: &'a str,
    ) -> BoxFuture<'a, Result<Option<RenderedSlugEntity>, ServerError>> {
        Box::pin(async move { Ok(self.rendered_slugs.lock().await.get(slug).cloned()) })
    }

    fn upsert_run<'a>(
        &'a self,
        run: WorkflowRunEntity,
    ) -> BoxFuture<'a, Result<WorkflowRunEntity, ServerError>> {
        Box::pin(async move {
            self.runs
                .lock()
                .await
                .insert(run.run_id.clone(), run.clone());
            Ok(run)
        })
    }

    fn find_run<'a>(
        &'a self,
        run_id: &'a str,
    ) -> BoxFuture<'a, Result<Option<WorkflowRunEntity>, ServerError>> {
        Box::pin(async move { Ok(self.runs.lock().await.get(run_id).cloned()) })
    }

    fn list_stale_running_runs<'a>(
        &'a self,
        current_boot_id: &'a str,
    ) -> BoxFuture<'a, Result<Vec<WorkflowRunEntity>, ServerError>> {
        Box::pin(async move {
            Ok(self
                .runs
                .lock()
                .await
                .values()
                .filter(|r| r.status == WorkflowRunStatus::Running && r.boot_id != current_boot_id)
                .cloned()
                .collect())
        })
    }
}
