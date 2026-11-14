//! Durable A2A task repository (PostgreSQL + in-memory test implementation).

use std::{collections::HashMap, future::Future, pin::Pin, sync::Arc};

use a2a::Task;
use sqlx::{PgPool, Row};
use tokio::sync::RwLock;

use crate::infra::error::ServerError;

pub type A2aRepoFuture<'a, T> = Pin<Box<dyn Future<Output = Result<T, ServerError>> + Send + 'a>>;

/// Persistence port for durable A2A tasks.
pub trait A2aRepository: Send + Sync {
    fn save_task(
        &self,
        task: Task,
        caller_org: Option<String>,
        caller_installation_id: Option<String>,
    ) -> A2aRepoFuture<'_, Task>;
    fn get_task(&self, task_id: &str) -> A2aRepoFuture<'_, Option<Task>>;
}

/// Injectable A2A task store wrapper.
#[derive(Clone)]
pub struct A2aTaskStore(pub Arc<dyn A2aRepository>);

impl Default for A2aTaskStore {
    fn default() -> Self {
        Self(Arc::new(InMemoryA2aRepository::default()))
    }
}

/// In-memory A2A task store for unit and integration tests.
#[derive(Default)]
pub struct InMemoryA2aRepository {
    tasks: RwLock<HashMap<String, Task>>,
}

impl A2aRepository for InMemoryA2aRepository {
    fn save_task(
        &self,
        task: Task,
        _caller_org: Option<String>,
        _caller_installation_id: Option<String>,
    ) -> A2aRepoFuture<'_, Task> {
        Box::pin(async move {
            self.tasks
                .write()
                .await
                .insert(task.id.clone(), task.clone());
            Ok(task)
        })
    }

    fn get_task(&self, task_id: &str) -> A2aRepoFuture<'_, Option<Task>> {
        let id = task_id.to_owned();
        Box::pin(async move { Ok(self.tasks.read().await.get(&id).cloned()) })
    }
}

/// PostgreSQL-backed A2A task store.
pub struct PgA2aRepository {
    pool: PgPool,
}

impl PgA2aRepository {
    #[must_use]
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

impl A2aRepository for PgA2aRepository {
    fn save_task(
        &self,
        task: Task,
        caller_org: Option<String>,
        caller_installation_id: Option<String>,
    ) -> A2aRepoFuture<'_, Task> {
        Box::pin(async move {
            let task_json = serde_json::to_value(&task)
                .map_err(|e| ServerError::Internal(format!("a2a task serialize failed: {e}")))?;
            let state_str = serde_json::to_value(&task.status.state)
                .ok()
                .and_then(|v| v.as_str().map(str::to_owned))
                .unwrap_or_else(|| "submitted".to_owned());

            sqlx::query(
                "INSERT INTO a2a_tasks \
                 (id, context_id, status_state, task_json, caller_org, caller_installation_id, updated_at) \
                 VALUES ($1, $2, $3, $4, $5, $6, NOW()) \
                 ON CONFLICT (id) DO UPDATE SET \
                 status_state = EXCLUDED.status_state, \
                 task_json = EXCLUDED.task_json, \
                 updated_at = NOW()",
            )
            .bind(&task.id)
            .bind(&task.context_id)
            .bind(&state_str)
            .bind(&task_json)
            .bind(&caller_org)
            .bind(&caller_installation_id)
            .execute(&self.pool)
            .await
            .map_err(|e| ServerError::Internal(format!("a2a task save failed: {e}")))?;

            Ok(task)
        })
    }

    fn get_task(&self, task_id: &str) -> A2aRepoFuture<'_, Option<Task>> {
        let id = task_id.to_owned();
        Box::pin(async move {
            let row = sqlx::query("SELECT task_json FROM a2a_tasks WHERE id = $1")
                .bind(&id)
                .fetch_optional(&self.pool)
                .await
                .map_err(|e| ServerError::Internal(format!("a2a task fetch failed: {e}")))?;

            match row {
                Some(r) => {
                    let val: serde_json::Value = r.get("task_json");
                    let task: Task = serde_json::from_value(val).map_err(|e| {
                        ServerError::Internal(format!("a2a task parse failed: {e}"))
                    })?;
                    Ok(Some(task))
                }
                None => Ok(None),
            }
        })
    }
}
