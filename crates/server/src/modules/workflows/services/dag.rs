//! Topological DAG validation and conditional edge evaluation.

use std::collections::{HashMap, HashSet, VecDeque};

use agents_protocol::{WorkflowEdge, WorkflowNode};
use serde_json::Value;

use crate::infra::error::ServerError;

/// Validates acyclic structure and produces a topological execution order.
///
/// # Errors
///
/// Returns [`ServerError::Validation`] when cycles or dangling edge references are found.
pub fn validate_dag_topological(
    nodes: &[WorkflowNode],
    edges: &[WorkflowEdge],
) -> Result<Vec<String>, ServerError> {
    let mut in_degree: HashMap<String, usize> = HashMap::new();
    let mut adj: HashMap<String, Vec<String>> = HashMap::new();
    let mut seen_ids: HashSet<&str> = HashSet::new();

    for node in nodes {
        let id = node.node_id.trim();
        if id.is_empty() {
            return Err(ServerError::Validation("node_id cannot be empty".into()));
        }
        if !seen_ids.insert(id) {
            return Err(ServerError::Validation(format!(
                "duplicate node_id `{id}` in DAG"
            )));
        }
        in_degree.insert(id.to_owned(), 0);
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
