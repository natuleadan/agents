//! PostgreSQL workflow and run checkpoint repository.

use std::sync::Arc;

use agents_protocol::{WorkflowEdge, WorkflowNode};
use sqlx::{PgPool, Row};

use crate::{
    infra::{error::ServerError, state::BoxFuture},
    modules::workflows::entities::{
        RenderedSlugEntity, WorkflowEntity, WorkflowRunEntity, WorkflowRunStatus,
    },
    modules::workflows::repositories::WorkflowRepository,
};

/// PostgreSQL workflow and run checkpoint repository.
#[derive(Clone)]
pub struct PgWorkflowRepository {
    pool: PgPool,
}

impl PgWorkflowRepository {
    #[must_use]
    pub fn new(pool: PgPool) -> Arc<Self> {
        Arc::new(Self { pool })
    }
}

impl WorkflowRepository for PgWorkflowRepository {
    fn create_workflow<'a>(
        &'a self,
        wf: WorkflowEntity,
    ) -> BoxFuture<'a, Result<WorkflowEntity, ServerError>> {
        Box::pin(async move {
            let nodes_json = serde_json::to_value(&wf.nodes)
                .map_err(|e| ServerError::Internal(format!("serialize nodes: {e}")))?;
            let edges_json = serde_json::to_value(&wf.edges)
                .map_err(|e| ServerError::Internal(format!("serialize edges: {e}")))?;
            sqlx::query(
                "INSERT INTO workflows (workflow_id, owner_id, name, nodes, edges, webhook_slug, render_slug, approved, approved_by, approved_at, created_by_agent, created_at, updated_at)
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13)",
            )
            .bind(&wf.workflow_id)
            .bind(&wf.owner_id)
            .bind(&wf.name)
            .bind(nodes_json)
            .bind(edges_json)
            .bind(&wf.webhook_slug)
            .bind(&wf.render_slug)
            .bind(wf.approved)
            .bind(&wf.approved_by)
            .bind(wf.approved_at.map(|v| v as i64))
            .bind(wf.created_by_agent)
            .bind(wf.created_at as i64)
            .bind(wf.updated_at as i64)
            .execute(&self.pool)
            .await
            .map_err(|e| ServerError::Internal(format!("create workflow failed: {e}")))?;
            Ok(wf)
        })
    }

    fn find_workflow<'a>(
        &'a self,
        workflow_id: &'a str,
    ) -> BoxFuture<'a, Result<Option<WorkflowEntity>, ServerError>> {
        Box::pin(async move {
            let row = sqlx::query(
                "SELECT workflow_id, owner_id, name, nodes, edges, webhook_slug, render_slug, approved, approved_by, approved_at, created_by_agent, created_at, updated_at
                 FROM workflows WHERE workflow_id = $1",
            )
            .bind(workflow_id)
            .fetch_optional(&self.pool)
            .await
            .map_err(|e| ServerError::Internal(format!("find workflow failed: {e}")))?;
            row.map(row_to_workflow).transpose()
        })
    }

    fn list_workflows_for_owner<'a>(
        &'a self,
        owner_id: &'a str,
    ) -> BoxFuture<'a, Result<Vec<WorkflowEntity>, ServerError>> {
        Box::pin(async move {
            let rows = sqlx::query(
                "SELECT workflow_id, owner_id, name, nodes, edges, webhook_slug, render_slug, approved, approved_by, approved_at, created_by_agent, created_at, updated_at
                 FROM workflows WHERE owner_id = $1 ORDER BY updated_at DESC",
            )
            .bind(owner_id)
            .fetch_all(&self.pool)
            .await
            .map_err(|e| ServerError::Internal(format!("list workflows failed: {e}")))?;
            rows.into_iter().map(row_to_workflow).collect()
        })
    }

    fn find_workflow_by_webhook_slug<'a>(
        &'a self,
        slug: &'a str,
    ) -> BoxFuture<'a, Result<Option<WorkflowEntity>, ServerError>> {
        Box::pin(async move {
            let row = sqlx::query(
                "SELECT workflow_id, owner_id, name, nodes, edges, webhook_slug, render_slug, approved, approved_by, approved_at, created_by_agent, created_at, updated_at
                 FROM workflows WHERE webhook_slug = $1",
            )
            .bind(slug)
            .fetch_optional(&self.pool)
            .await
            .map_err(|e| ServerError::Internal(format!("find workflow by webhook slug failed: {e}")))?;
            row.map(row_to_workflow).transpose()
        })
    }

    fn find_workflow_by_render_slug<'a>(
        &'a self,
        slug: &'a str,
    ) -> BoxFuture<'a, Result<Option<WorkflowEntity>, ServerError>> {
        Box::pin(async move {
            let row = sqlx::query(
                "SELECT workflow_id, owner_id, name, nodes, edges, webhook_slug, render_slug, approved, approved_by, approved_at, created_by_agent, created_at, updated_at
                 FROM workflows WHERE render_slug = $1",
            )
            .bind(slug)
            .fetch_optional(&self.pool)
            .await
            .map_err(|e| ServerError::Internal(format!("find workflow by render slug failed: {e}")))?;
            row.map(row_to_workflow).transpose()
        })
    }

    fn approve_workflow<'a>(
        &'a self,
        workflow_id: &'a str,
        approver_id: &'a str,
        approved_at: u64,
    ) -> BoxFuture<'a, Result<WorkflowEntity, ServerError>> {
        Box::pin(async move {
            let row = sqlx::query(
                "UPDATE workflows
                 SET approved = TRUE, approved_by = $2, approved_at = $3, updated_at = $3
                 WHERE workflow_id = $1
                 RETURNING workflow_id, owner_id, name, nodes, edges, webhook_slug, render_slug, approved, approved_by, approved_at, created_by_agent, created_at, updated_at",
            )
            .bind(workflow_id)
            .bind(approver_id)
            .bind(approved_at as i64)
            .fetch_optional(&self.pool)
            .await
            .map_err(|e| ServerError::Internal(format!("approve workflow failed: {e}")))?
            .ok_or_else(|| ServerError::NotFound(format!("workflow `{workflow_id}` not found")))?;

            let wf = row_to_workflow(row)?;

            // If there's an associated render slug, approve it too
            if let Some(slug) = &wf.render_slug {
                let _ = sqlx::query(
                    "UPDATE rendered_slugs SET approved = TRUE, updated_at = $2 WHERE slug = $1",
                )
                .bind(slug)
                .bind(approved_at as i64)
                .execute(&self.pool)
                .await;
            }

            Ok(wf)
        })
    }

    fn upsert_rendered_slug<'a>(
        &'a self,
        entity: RenderedSlugEntity,
    ) -> BoxFuture<'a, Result<RenderedSlugEntity, ServerError>> {
        Box::pin(async move {
            sqlx::query(
                "INSERT INTO rendered_slugs (slug, workflow_id, content_type, content, approved, updated_at)
                 VALUES ($1, $2, $3, $4, $5, $6)
                 ON CONFLICT (slug) DO UPDATE
                 SET content_type = EXCLUDED.content_type, content = EXCLUDED.content, approved = EXCLUDED.approved, updated_at = EXCLUDED.updated_at",
            )
            .bind(&entity.slug)
            .bind(&entity.workflow_id)
            .bind(&entity.content_type)
            .bind(&entity.content)
            .bind(entity.approved)
            .bind(entity.updated_at as i64)
            .execute(&self.pool)
            .await
            .map_err(|e| ServerError::Internal(format!("upsert rendered slug failed: {e}")))?;
            Ok(entity)
        })
    }

    fn find_rendered_slug<'a>(
        &'a self,
        slug: &'a str,
    ) -> BoxFuture<'a, Result<Option<RenderedSlugEntity>, ServerError>> {
        Box::pin(async move {
            let row = sqlx::query(
                "SELECT slug, workflow_id, content_type, content, approved, updated_at
                 FROM rendered_slugs WHERE slug = $1",
            )
            .bind(slug)
            .fetch_optional(&self.pool)
            .await
            .map_err(|e| ServerError::Internal(format!("find rendered slug failed: {e}")))?;

            Ok(row.map(|r| RenderedSlugEntity {
                slug: r.get("slug"),
                workflow_id: r.get("workflow_id"),
                content_type: r.get("content_type"),
                content: r.get("content"),
                approved: r.get("approved"),
                updated_at: r.get::<i64, _>("updated_at").max(0) as u64,
            }))
        })
    }

    fn upsert_run<'a>(
        &'a self,
        run: WorkflowRunEntity,
    ) -> BoxFuture<'a, Result<WorkflowRunEntity, ServerError>> {
        Box::pin(async move {
            sqlx::query(
                "INSERT INTO workflow_runs (run_id, workflow_id, schedule_id, owner_id, boot_id, status, current_node_id, checkpoint_state, resumed_count, started_at, completed_at)
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)
                 ON CONFLICT (run_id) DO UPDATE
                 SET boot_id = EXCLUDED.boot_id, status = EXCLUDED.status, current_node_id = EXCLUDED.current_node_id, checkpoint_state = EXCLUDED.checkpoint_state, resumed_count = EXCLUDED.resumed_count, completed_at = EXCLUDED.completed_at",
            )
            .bind(&run.run_id)
            .bind(&run.workflow_id)
            .bind(&run.schedule_id)
            .bind(&run.owner_id)
            .bind(&run.boot_id)
            .bind(run.status.as_str())
            .bind(&run.current_node_id)
            .bind(&run.checkpoint_state)
            .bind(run.resumed_count as i32)
            .bind(run.started_at as i64)
            .bind(run.completed_at.map(|v| v as i64))
            .execute(&self.pool)
            .await
            .map_err(|e| ServerError::Internal(format!("upsert workflow run failed: {e}")))?;
            Ok(run)
        })
    }

    fn find_run<'a>(
        &'a self,
        run_id: &'a str,
    ) -> BoxFuture<'a, Result<Option<WorkflowRunEntity>, ServerError>> {
        Box::pin(async move {
            let row = sqlx::query(
                "SELECT run_id, workflow_id, schedule_id, owner_id, boot_id, status, current_node_id, checkpoint_state, resumed_count, started_at, completed_at
                 FROM workflow_runs WHERE run_id = $1",
            )
            .bind(run_id)
            .fetch_optional(&self.pool)
            .await
            .map_err(|e| ServerError::Internal(format!("find workflow run failed: {e}")))?;
            row.map(row_to_run).transpose()
        })
    }

    fn list_stale_running_runs<'a>(
        &'a self,
        current_boot_id: &'a str,
    ) -> BoxFuture<'a, Result<Vec<WorkflowRunEntity>, ServerError>> {
        Box::pin(async move {
            let rows = sqlx::query(
                "SELECT run_id, workflow_id, schedule_id, owner_id, boot_id, status, current_node_id, checkpoint_state, resumed_count, started_at, completed_at
                 FROM workflow_runs WHERE status = 'running' AND boot_id != $1",
            )
            .bind(current_boot_id)
            .fetch_all(&self.pool)
            .await
            .map_err(|e| ServerError::Internal(format!("list stale runs failed: {e}")))?;
            rows.into_iter().map(row_to_run).collect()
        })
    }
}

fn row_to_workflow(row: sqlx::postgres::PgRow) -> Result<WorkflowEntity, ServerError> {
    let nodes: Vec<WorkflowNode> = serde_json::from_value(row.get("nodes"))
        .map_err(|e| ServerError::Internal(format!("corrupt workflow nodes: {e}")))?;
    let edges: Vec<WorkflowEdge> = serde_json::from_value(row.get("edges"))
        .map_err(|e| ServerError::Internal(format!("corrupt workflow edges: {e}")))?;
    Ok(WorkflowEntity {
        workflow_id: row.get("workflow_id"),
        owner_id: row.get("owner_id"),
        name: row.get("name"),
        nodes,
        edges,
        webhook_slug: row.try_get("webhook_slug").ok().flatten(),
        render_slug: row.try_get("render_slug").ok().flatten(),
        approved: row.try_get("approved").unwrap_or(true),
        approved_by: row.try_get("approved_by").ok().flatten(),
        approved_at: row
            .try_get::<Option<i64>, _>("approved_at")
            .ok()
            .flatten()
            .map(|v| v.max(0) as u64),
        created_by_agent: row.try_get("created_by_agent").unwrap_or(false),
        created_at: row.get::<i64, _>("created_at").max(0) as u64,
        updated_at: row.get::<i64, _>("updated_at").max(0) as u64,
    })
}

fn row_to_run(row: sqlx::postgres::PgRow) -> Result<WorkflowRunEntity, ServerError> {
    let status_raw: String = row.get("status");
    Ok(WorkflowRunEntity {
        run_id: row.get("run_id"),
        workflow_id: row.get("workflow_id"),
        schedule_id: row.get("schedule_id"),
        owner_id: row.get("owner_id"),
        boot_id: row.get("boot_id"),
        status: WorkflowRunStatus::parse(&status_raw),
        current_node_id: row.get("current_node_id"),
        checkpoint_state: row.get("checkpoint_state"),
        resumed_count: row.get::<i32, _>("resumed_count").max(0) as u32,
        started_at: row.get::<i64, _>("started_at").max(0) as u64,
        completed_at: row
            .get::<Option<i64>, _>("completed_at")
            .map(|v| v.max(0) as u64),
    })
}
