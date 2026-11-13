//! Persistence repository for DAG workflows and crash-resumable workflow runs.

use std::{collections::HashMap, sync::Arc};

use agents_protocol::{WorkflowEdge, WorkflowNode};
use sqlx::{PgPool, Row};
use tokio::sync::Mutex;

use crate::{
    infra::{error::ServerError, state::BoxFuture},
    modules::workflows::entities::{WorkflowEntity, WorkflowRunEntity, WorkflowRunStatus},
};

/// Concrete DI wrapper around [`WorkflowRepository`].
#[derive(Clone)]
pub struct WorkflowStore(pub Arc<dyn WorkflowRepository>);

impl Default for WorkflowStore {
    fn default() -> Self {
        Self(InMemoryWorkflowRepository::new())
    }
}

/// Persistence contract for workflows and checkpointed execution runs.
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
                "INSERT INTO workflows (workflow_id, owner_id, name, nodes, edges, created_at, updated_at)
                 VALUES ($1, $2, $3, $4, $5, $6, $7)",
            )
            .bind(&wf.workflow_id)
            .bind(&wf.owner_id)
            .bind(&wf.name)
            .bind(nodes_json)
            .bind(edges_json)
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
                "SELECT workflow_id, owner_id, name, nodes, edges, created_at, updated_at
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
                "SELECT workflow_id, owner_id, name, nodes, edges, created_at, updated_at
                 FROM workflows WHERE owner_id = $1 ORDER BY updated_at DESC",
            )
            .bind(owner_id)
            .fetch_all(&self.pool)
            .await
            .map_err(|e| ServerError::Internal(format!("list workflows failed: {e}")))?;
            rows.into_iter().map(row_to_workflow).collect()
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
                 ON CONFLICT (run_id) DO UPDATE SET
                   boot_id = EXCLUDED.boot_id,
                   status = EXCLUDED.status,
                   current_node_id = EXCLUDED.current_node_id,
                   checkpoint_state = EXCLUDED.checkpoint_state,
                   resumed_count = EXCLUDED.resumed_count,
                   completed_at = EXCLUDED.completed_at",
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
                 FROM workflow_runs WHERE status = 'running' AND boot_id <> $1",
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

/// Deterministic in-memory workflow and run checkpoint repository.
#[derive(Clone, Default)]
pub struct InMemoryWorkflowRepository {
    workflows: Arc<Mutex<HashMap<String, WorkflowEntity>>>,
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
