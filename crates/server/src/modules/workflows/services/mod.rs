//! DAG validation, conditional branching execution, and crash-resume checkpoint recovery.

use std::{
    collections::{HashMap, HashSet, VecDeque},
    sync::Arc,
};

use agents_protocol::{ExecOptions, InteractionMode, TaskRequest, WorkflowEdge, WorkflowNode};
use ironic::prelude::*;
use serde_json::{Value, json};

use crate::{
    infra::{error::ServerError, state::now_ms, stream::BusClient},
    modules::{
        audit::services::AuditService,
        auth::{entities::AuthenticatedActor, guards::ensure_owner},
        workflows::{
            dto::CreateWorkflowDto,
            entities::{WorkflowEntity, WorkflowRunEntity, WorkflowRunStatus},
            repositories::WorkflowStore,
        },
    },
};

/// Orchestrates node-based DAG workflows and checkpointed crash recovery.
#[derive(Injectable)]
pub struct WorkflowService {
    store: Arc<WorkflowStore>,
    bus: Arc<BusClient>,
    audit: Arc<AuditService>,
}

impl WorkflowService {
    /// Validates an acyclic DAG definition and persists it for `actor`.
    ///
    /// # Errors
    ///
    /// Returns [`ServerError::Validation`] on cycles, duplicate IDs, or dangling edges.
    pub async fn create(
        &self,
        actor: &AuthenticatedActor,
        dto: &CreateWorkflowDto,
    ) -> Result<WorkflowEntity, ServerError> {
        let _ = validate_dag_topological(&dto.nodes, &dto.edges)?;
        let now = now_ms();
        let entity = WorkflowEntity {
            workflow_id: uuid::Uuid::new_v4().to_string(),
            owner_id: actor.user_id.clone(),
            name: dto.name.trim().to_owned(),
            nodes: dto.nodes.clone(),
            edges: dto.edges.clone(),
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
                json!({ "nodes": created.nodes.len(), "edges": created.edges.len() }),
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
    ///
    /// # Errors
    ///
    /// Returns [`ServerError::NotFound`] or [`ServerError::Forbidden`].
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
    ///
    /// # Errors
    ///
    /// Returns [`ServerError::NotFound`] if the workflow does not exist.
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

    /// Executes a loaded workflow entity (used by both controller and Autocron scheduler).
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
    ///
    /// # Errors
    ///
    /// Returns [`ServerError`] if checkpoint reads or resumed steps fail.
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

            let incoming: Vec<&WorkflowEdge> = wf
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
            let node_out = self.execute_node(&run, node, &outputs).await?;
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

    async fn execute_node(
        &self,
        run: &WorkflowRunEntity,
        node: &WorkflowNode,
        _outputs: &serde_json::Map<String, Value>,
    ) -> Result<Value, ServerError> {
        match node.kind.as_str() {
            "agent_step" => {
                let prompt = node
                    .config
                    .get("prompt")
                    .and_then(Value::as_str)
                    .unwrap_or("execute workflow step")
                    .to_owned();
                let req = TaskRequest {
                    task_id: format!("{}-{}", run.run_id, node.node_id),
                    session_id: format!("wf-{}", run.workflow_id),
                    prompt,
                    timeout_seconds: Some(120),
                    mode: InteractionMode::Act,
                    fencing_token: None,
                    idempotency_key: Some(format!("wf:{}:{}", run.run_id, node.node_id)),
                    sandbox: None,
                    exec: ExecOptions::default(),
                };
                self.bus.0.publish_task_request(&req).await?;
                Ok(json!({ "dispatched": true, "taskId": req.task_id }))
            }
            "condition" => {
                let result = node
                    .config
                    .get("branch")
                    .cloned()
                    .unwrap_or(Value::Bool(true));
                Ok(json!({ "branch": result }))
            }
            _ => Ok(json!({ "ok": true, "kind": node.kind })),
        }
    }
}

/// Validates node uniqueness, edge targets, and acyclic structure using Kahn's algorithm.
///
/// # Errors
///
/// Returns [`ServerError::Validation`] if empty, cyclic, or referencing unknown nodes.
pub fn validate_dag_topological(
    nodes: &[WorkflowNode],
    edges: &[WorkflowEdge],
) -> Result<Vec<String>, ServerError> {
    if nodes.is_empty() {
        return Err(ServerError::Validation(
            "workflow must contain at least one node".into(),
        ));
    }

    let mut in_degree: HashMap<String, usize> = HashMap::new();
    let mut adj: HashMap<String, Vec<String>> = HashMap::new();

    for node in nodes {
        let id = node.node_id.trim();
        if id.is_empty() {
            return Err(ServerError::Validation("node_id must not be empty".into()));
        }
        if in_degree.insert(id.to_owned(), 0).is_some() {
            return Err(ServerError::Validation(format!(
                "duplicate workflow node_id `{id}`"
            )));
        }
        adj.insert(id.to_owned(), Vec::new());
    }

    for edge in edges {
        if !in_degree.contains_key(&edge.from_node_id) {
            return Err(ServerError::Validation(format!(
                "edge references unknown source node `{}`",
                edge.from_node_id
            )));
        }
        if !in_degree.contains_key(&edge.to_node_id) {
            return Err(ServerError::Validation(format!(
                "edge references unknown target node `{}`",
                edge.to_node_id
            )));
        }
        if let Some(list) = adj.get_mut(&edge.from_node_id) {
            list.push(edge.to_node_id.clone());
        }
        if let Some(deg) = in_degree.get_mut(&edge.to_node_id) {
            *deg += 1;
        }
    }

    let mut queue: VecDeque<String> = nodes
        .iter()
        .map(|n| n.node_id.clone())
        .filter(|id| in_degree.get(id).copied().unwrap_or(0) == 0)
        .collect();

    let mut ordered = Vec::with_capacity(nodes.len());
    while let Some(curr) = queue.pop_front() {
        ordered.push(curr.clone());
        if let Some(neighbors) = adj.get(&curr) {
            for next in neighbors {
                if let Some(deg) = in_degree.get_mut(next) {
                    *deg = deg.saturating_sub(1);
                    if *deg == 0 {
                        queue.push_back(next.clone());
                    }
                }
            }
        }
    }

    if ordered.len() != nodes.len() {
        return Err(ServerError::Validation(
            "workflow DAG contains a cycle; cyclic graphs are forbidden".into(),
        ));
    }

    Ok(ordered)
}

/// Evaluates simple conditional edge expressions (`key == val` or `key != val` or `true`).
#[must_use]
pub fn evaluate_edge_condition(condition: Option<&str>, prev_output: Option<&Value>) -> bool {
    let Some(cond) = condition.map(str::trim).filter(|c| !c.is_empty()) else {
        return true;
    };
    if let Some((lhs, rhs)) = cond.split_once("==") {
        let key = lhs.trim();
        let expected = rhs.trim().trim_matches('"').trim_matches('\'');
        let actual = prev_output
            .and_then(|v| v.get(key))
            .map(value_to_scalar)
            .unwrap_or_default();
        return actual == expected;
    }
    if let Some((lhs, rhs)) = cond.split_once("!=") {
        let key = lhs.trim();
        let expected = rhs.trim().trim_matches('"').trim_matches('\'');
        let actual = prev_output
            .and_then(|v| v.get(key))
            .map(value_to_scalar)
            .unwrap_or_default();
        return actual != expected;
    }
    prev_output
        .and_then(|v| v.get(cond))
        .and_then(Value::as_bool)
        .unwrap_or(false)
}

fn value_to_scalar(val: &Value) -> String {
    match val {
        Value::String(s) => s.clone(),
        Value::Bool(b) => b.to_string(),
        Value::Number(n) => n.to_string(),
        _ => String::new(),
    }
}
