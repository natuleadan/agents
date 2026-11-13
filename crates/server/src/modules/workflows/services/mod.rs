//! DAG validation, conditional branching execution, and crash-resume checkpoint recovery.

pub mod dag;
pub mod nodes;

use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
};

use agents_protocol::WorkflowNode;
use ironic::prelude::*;
use serde_json::{Value, json};

pub use dag::{evaluate_edge_condition, validate_dag_topological};
pub use nodes::execute_single_node;

use crate::{
    infra::{error::ServerError, state::now_ms, stream::BusClient},
    modules::{
        audit::services::AuditService,
        auth::{entities::AuthenticatedActor, guards::ensure_owner},
        workflows::{
            dto::{CreateWorkflowDto, PublishRenderSlugDto},
            entities::{RenderedSlugEntity, WorkflowEntity, WorkflowRunEntity, WorkflowRunStatus},
            repositories::WorkflowStore,
        },
    },
};

/// Orchestrates node-based DAG workflows, webhook triggers, dynamic slugs, and checkpoints.
#[derive(Injectable)]
pub struct WorkflowService {
    store: Arc<WorkflowStore>,
    bus: Arc<BusClient>,
    audit: Arc<AuditService>,
}

impl WorkflowService {
    /// Validates an acyclic DAG definition and persists it for `actor`.
    pub async fn create(
        &self,
        actor: &AuthenticatedActor,
        dto: &CreateWorkflowDto,
    ) -> Result<WorkflowEntity, ServerError> {
        let _ = validate_dag_topological(&dto.nodes, &dto.edges)?;
        let now = now_ms();
        let is_agent = dto.created_by_agent.unwrap_or(false);
        // OWASP ASI-02/ASI-08: Workflows created by agents require supervisor approval
        let approved = !is_agent;

        let entity = WorkflowEntity {
            workflow_id: uuid::Uuid::new_v4().to_string(),
            owner_id: actor.user_id.clone(),
            name: dto.name.trim().to_owned(),
            nodes: dto.nodes.clone(),
            edges: dto.edges.clone(),
            webhook_slug: dto.webhook_slug.as_ref().map(|s| s.trim().to_owned()),
            render_slug: dto.render_slug.as_ref().map(|s| s.trim().to_owned()),
            approved,
            approved_by: if approved {
                Some(actor.user_id.clone())
            } else {
                None
            },
            approved_at: if approved { Some(now) } else { None },
            created_by_agent: is_agent,
            created_at: now,
            updated_at: now,
        };

        let created = self.store.0.create_workflow(entity).await?;
        let _ = self
            .audit
            .record(
                Some(&actor.user_id),
                "workflow.create",
                "workflow",
                Some(&created.workflow_id),
                "success",
                json!({
                    "nodes": created.nodes.len(),
                    "edges": created.edges.len(),
                    "approved": created.approved,
                    "createdByAgent": is_agent,
                }),
            )
            .await;
        Ok(created)
    }

    /// Lists workflows owned by `actor`.
    pub async fn list_owned(
        &self,
        actor: &AuthenticatedActor,
    ) -> Result<Vec<WorkflowEntity>, ServerError> {
        self.store.0.list_workflows_for_owner(&actor.user_id).await
    }

    /// Loads an owned workflow with strict BOLA check.
    pub async fn get_owned(
        &self,
        actor: &AuthenticatedActor,
        workflow_id: &str,
    ) -> Result<WorkflowEntity, ServerError> {
        let wf = self
            .store
            .0
            .find_workflow(workflow_id)
            .await?
            .ok_or_else(|| ServerError::NotFound(format!("workflow `{workflow_id}` not found")))?;
        ensure_owner(actor, &wf.owner_id).map_err(|_| {
            ServerError::Forbidden("Access denied: caller does not own target workflow".into())
        })?;
        Ok(wf)
    }

    /// Approves an agent-created or pending workflow DAG (OWASP ASI-02/ASI-08).
    pub async fn approve(
        &self,
        actor: &AuthenticatedActor,
        workflow_id: &str,
    ) -> Result<WorkflowEntity, ServerError> {
        let wf = self.get_owned(actor, workflow_id).await?;
        let approved = self
            .store
            .0
            .approve_workflow(&wf.workflow_id, &actor.user_id, now_ms())
            .await?;
        let _ = self
            .audit
            .record(
                Some(&actor.user_id),
                "workflow.approve",
                "workflow",
                Some(&approved.workflow_id),
                "success",
                json!({ "approved": true }),
            )
            .await;
        Ok(approved)
    }

    /// Publishes content to the workflow's configured render slug.
    pub async fn publish_render(
        &self,
        actor: &AuthenticatedActor,
        workflow_id: &str,
        dto: &PublishRenderSlugDto,
    ) -> Result<RenderedSlugEntity, ServerError> {
        let wf = self.get_owned(actor, workflow_id).await?;
        let slug = wf.render_slug.as_deref().ok_or_else(|| {
            ServerError::Validation("workflow does not have a render_slug configured".into())
        })?;
        let content_type = dto
            .content_type
            .as_deref()
            .unwrap_or("text/html; charset=utf-8")
            .to_owned();
        let entity = RenderedSlugEntity {
            slug: slug.to_owned(),
            workflow_id: wf.workflow_id.clone(),
            content_type,
            content: dto.content.clone(),
            approved: wf.approved,
            updated_at: now_ms(),
        };
        let saved = self.store.0.upsert_rendered_slug(entity).await?;
        let _ = self
            .audit
            .record(
                Some(&actor.user_id),
                "workflow.render_publish",
                "workflow",
                Some(&wf.workflow_id),
                "success",
                json!({ "slug": slug, "approved": wf.approved }),
            )
            .await;
        Ok(saved)
    }

    /// Retrieves rendered slug content for public/authenticated viewing.
    pub async fn get_render(&self, slug: &str) -> Result<RenderedSlugEntity, ServerError> {
        let rendered = self
            .store
            .0
            .find_rendered_slug(slug)
            .await?
            .ok_or_else(|| ServerError::NotFound(format!("render slug `{slug}` not found")))?;
        if !rendered.approved {
            return Err(ServerError::Forbidden(
                "slug pending supervisor approval (OWASP ASI-02/ASI-08)".into(),
            ));
        }
        Ok(rendered)
    }

    /// Triggers a workflow by its dynamic webhook slug.
    pub async fn trigger_by_webhook_slug(
        &self,
        slug: &str,
        payload: Value,
    ) -> Result<WorkflowRunEntity, ServerError> {
        let wf = self
            .store
            .0
            .find_workflow_by_webhook_slug(slug)
            .await?
            .ok_or_else(|| ServerError::NotFound(format!("webhook slug `{slug}` not found")))?;
        if !wf.approved {
            return Err(ServerError::Forbidden(
                "workflow pending supervisor approval (OWASP ASI-02/ASI-08)".into(),
            ));
        }
        let run = self
            .execute_workflow_internal(&wf, "webhook-ingress", None, payload)
            .await?;
        let _ = self
            .audit
            .record(
                None,
                "workflow.webhook_trigger",
                "workflow",
                Some(&wf.workflow_id),
                "success",
                json!({ "slug": slug, "runId": run.run_id }),
            )
            .await;
        Ok(run)
    }

    /// Executes an owned workflow DAG from the start with checkpoint persistence.
    pub async fn execute_owned(
        &self,
        actor: &AuthenticatedActor,
        workflow_id: &str,
        boot_id: &str,
        schedule_id: Option<&str>,
        input: Value,
    ) -> Result<WorkflowRunEntity, ServerError> {
        let wf = self.get_owned(actor, workflow_id).await?;
        self.execute_workflow_internal(&wf, boot_id, schedule_id, input)
            .await
    }

    /// Loads a workflow without BOLA check for trusted internal Autocron dispatch.
    pub async fn get_by_id_internal(
        &self,
        workflow_id: &str,
    ) -> Result<WorkflowEntity, ServerError> {
        self.store
            .0
            .find_workflow(workflow_id)
            .await?
            .ok_or_else(|| ServerError::NotFound(format!("workflow `{workflow_id}` not found")))
    }

    /// Executes a loaded workflow entity (used by controller, webhooks, and Autocron).
    pub async fn execute_workflow_internal(
        &self,
        wf: &WorkflowEntity,
        boot_id: &str,
        schedule_id: Option<&str>,
        input: Value,
    ) -> Result<WorkflowRunEntity, ServerError> {
        let run = WorkflowRunEntity {
            run_id: uuid::Uuid::new_v4().to_string(),
            workflow_id: wf.workflow_id.clone(),
            schedule_id: schedule_id.map(ToOwned::to_owned),
            owner_id: wf.owner_id.clone(),
            boot_id: boot_id.to_owned(),
            status: WorkflowRunStatus::Running,
            current_node_id: None,
            checkpoint_state: json!({
                "input": input,
                "completedNodes": [],
                "outputs": {},
            }),
            resumed_count: 0,
            started_at: now_ms(),
            completed_at: None,
        };
        let saved = self.store.0.upsert_run(run).await?;
        self.run_from_checkpoint(wf, saved).await
    }

    /// Reconciles and resumes any `Running` workflow runs orphaned by a previous `boot_id`.
    pub async fn reconcile_interrupted_runs(
        &self,
        current_boot_id: &str,
    ) -> Result<u32, ServerError> {
        let stale_runs = self
            .store
            .0
            .list_stale_running_runs(current_boot_id)
            .await?;
        let mut resumed_total: u32 = 0;

        for mut run in stale_runs {
            let Some(wf) = self.store.0.find_workflow(&run.workflow_id).await? else {
                run.status = WorkflowRunStatus::Failed;
                run.completed_at = Some(now_ms());
                let _ = self.store.0.upsert_run(run).await?;
                continue;
            };
            run.boot_id = current_boot_id.to_owned();
            run.resumed_count = run.resumed_count.saturating_add(1);
            let updated = self.store.0.upsert_run(run).await?;
            let _ = self.run_from_checkpoint(&wf, updated).await?;
            resumed_total = resumed_total.saturating_add(1);
        }

        Ok(resumed_total)
    }

    /// Loads an owned workflow run checkpoint.
    pub async fn get_run_owned(
        &self,
        actor: &AuthenticatedActor,
        run_id: &str,
    ) -> Result<WorkflowRunEntity, ServerError> {
        let run =
            self.store.0.find_run(run_id).await?.ok_or_else(|| {
                ServerError::NotFound(format!("workflow run `{run_id}` not found"))
            })?;
        ensure_owner(actor, &run.owner_id).map_err(|_| {
            ServerError::Forbidden("Access denied: caller does not own target run".into())
        })?;
        Ok(run)
    }

    async fn run_from_checkpoint(
        &self,
        wf: &WorkflowEntity,
        mut run: WorkflowRunEntity,
    ) -> Result<WorkflowRunEntity, ServerError> {
        let topo_order = validate_dag_topological(&wf.nodes, &wf.edges)?;
        let node_map: HashMap<&str, &WorkflowNode> =
            wf.nodes.iter().map(|n| (n.node_id.as_str(), n)).collect();

        let mut completed: HashSet<String> = run
            .checkpoint_state
            .get("completedNodes")
            .and_then(Value::as_array)
            .map(|arr| {
                arr.iter()
                    .filter_map(Value::as_str)
                    .map(ToOwned::to_owned)
                    .collect()
            })
            .unwrap_or_default();

        let mut outputs: serde_json::Map<String, Value> = run
            .checkpoint_state
            .get("outputs")
            .and_then(Value::as_object)
            .cloned()
            .unwrap_or_default();

        for node_id in topo_order {
            if completed.contains(&node_id) {
                continue;
            }
            let Some(node) = node_map.get(node_id.as_str()) else {
                continue;
            };

            let incoming: Vec<&agents_protocol::WorkflowEdge> = wf
                .edges
                .iter()
                .filter(|e| e.to_node_id == node_id)
                .collect();

            if !incoming.is_empty() {
                let any_active = incoming.iter().any(|edge| {
                    completed.contains(&edge.from_node_id)
                        && evaluate_edge_condition(
                            edge.condition.as_deref(),
                            outputs.get(&edge.from_node_id),
                        )
                });
                if !any_active {
                    continue;
                }
            }

            run.current_node_id = Some(node_id.clone());
            let node_out = execute_single_node(&self.bus, &run, node, &outputs).await?;
            outputs.insert(node_id.clone(), node_out);
            completed.insert(node_id.clone());

            let completed_vec: Vec<Value> = completed.iter().cloned().map(Value::String).collect();
            run.checkpoint_state = json!({
                "input": run.checkpoint_state.get("input").cloned().unwrap_or(Value::Null),
                "completedNodes": completed_vec,
                "outputs": Value::Object(outputs.clone()),
            });
            run = self.store.0.upsert_run(run).await?;
        }

        run.status = WorkflowRunStatus::Completed;
        run.current_node_id = None;
        run.completed_at = Some(now_ms());
        self.store.0.upsert_run(run).await
    }
}
