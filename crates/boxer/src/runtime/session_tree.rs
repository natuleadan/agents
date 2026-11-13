//! Append-only branching session tree with atomic `(ToolCall, ToolResult)` compaction
//! boundaries, reversible file checkpoints, and memory provenance entries.

use std::collections::{BTreeMap, HashMap};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::config::now_ms;

/// Single immutable entry appended to a session branch.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum SessionEntry {
    UserMessage {
        text: String,
    },
    AssistantText {
        text: String,
    },
    ToolCall {
        call_id: String,
        tool_name: String,
        input: Value,
    },
    ToolResult {
        call_id: String,
        tool_name: String,
        output: String,
        is_error: bool,
    },
    CompactionSummary {
        compacted_nodes: usize,
        summary: String,
    },
    CheckpointRef {
        checkpoint_id: String,
        label: String,
    },
}

impl SessionEntry {
    #[must_use]
    pub fn role_label(&self) -> &'static str {
        match self {
            Self::UserMessage { .. } => "user",
            Self::AssistantText { .. } | Self::ToolCall { .. } => "assistant",
            Self::ToolResult { .. } => "tool",
            Self::CompactionSummary { .. } | Self::CheckpointRef { .. } => "system",
        }
    }

    #[must_use]
    pub fn summary_text(&self) -> String {
        match self {
            Self::UserMessage { text } | Self::AssistantText { text } => text.clone(),
            Self::ToolCall {
                call_id, tool_name, ..
            } => format!("[tool_call:{call_id}] {tool_name}"),
            Self::ToolResult {
                call_id,
                output,
                is_error,
                ..
            } => format!("[tool_result:{call_id}:err={is_error}] {output}"),
            Self::CompactionSummary { summary, .. } => summary.clone(),
            Self::CheckpointRef {
                checkpoint_id,
                label,
            } => format!("[checkpoint:{checkpoint_id}] {label}"),
        }
    }
}

/// Tree node supporting conversation branches and forks without mutating history.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionNode {
    pub node_id: String,
    pub parent_id: Option<String>,
    pub branch: String,
    pub created_at_ms: u64,
    pub entry: SessionEntry,
}

/// Named cursor pointing at the head of a session branch.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionBranch {
    pub name: String,
    pub head_node_id: Option<String>,
}

/// Reversible file/state snapshot allowing rollback inside an agent session.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FileCheckpoint {
    pub checkpoint_id: String,
    pub parent_node_id: Option<String>,
    pub label: String,
    pub snapshots: BTreeMap<String, Option<Vec<u8>>>,
    pub created_at_ms: u64,
}

/// Session memory / instinct item annotated with explicit provenance.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MemoryEntry {
    pub id: String,
    pub content: String,
    pub provenance_source: String,
    pub instinct_tag: Option<String>,
    pub confidence_pct: u8,
    pub recorded_at_ms: u64,
}

/// Append-only tree managing branches, atomic compaction, checkpoints, and memories.
#[derive(Debug, Clone)]
pub struct SessionTree {
    pub session_id: String,
    nodes: Vec<SessionNode>,
    node_index: HashMap<String, usize>,
    branches: HashMap<String, Option<String>>,
    active_branch: String,
    checkpoints: HashMap<String, FileCheckpoint>,
    memories: Vec<MemoryEntry>,
}

impl SessionTree {
    #[must_use]
    pub fn new(session_id: impl Into<String>) -> Self {
        let mut branches = HashMap::new();
        branches.insert("main".to_owned(), None);
        Self {
            session_id: session_id.into(),
            nodes: Vec::new(),
            node_index: HashMap::new(),
            branches,
            active_branch: "main".to_owned(),
            checkpoints: HashMap::new(),
            memories: Vec::new(),
        }
    }

    /// Appends a new entry onto the currently active branch and advances its head pointer.
    pub fn append(&mut self, entry: SessionEntry) -> String {
        let parent_id = self
            .branches
            .get(&self.active_branch)
            .and_then(|opt| opt.clone());
        let node_id = uuid::Uuid::new_v4().to_string();
        let node = SessionNode {
            node_id: node_id.clone(),
            parent_id,
            branch: self.active_branch.clone(),
            created_at_ms: now_ms(),
            entry,
        };
        let idx = self.nodes.len();
        self.nodes.push(node);
        self.node_index.insert(node_id.clone(), idx);
        self.branches
            .insert(self.active_branch.clone(), Some(node_id.clone()));
        node_id
    }

    /// Creates a new branch forked from an existing `from_node_id` (or active head) and switches to it.
    pub fn fork_branch(&mut self, branch_name: impl Into<String>, from_node_id: Option<&str>) {
        let name = branch_name.into();
        let base_head = from_node_id
            .map(ToOwned::to_owned)
            .or_else(|| self.active_head().map(ToOwned::to_owned));
        self.branches.insert(name.clone(), base_head);
        self.active_branch = name;
    }

    /// Switches active branch if registered.
    pub fn checkout_branch(&mut self, branch_name: &str) -> bool {
        if self.branches.contains_key(branch_name) {
            self.active_branch = branch_name.to_owned();
            return true;
        }
        false
    }

    #[must_use]
    pub fn active_head(&self) -> Option<&str> {
        self.branches
            .get(&self.active_branch)
            .and_then(|opt| opt.as_deref())
    }

    /// Walks root-to-head along the active branch returning ordered entries.
    #[must_use]
    pub fn active_entries(&self) -> Vec<SessionEntry> {
        let mut chain = Vec::new();
        let mut cursor = self.active_head().map(ToOwned::to_owned);
        while let Some(id) = cursor {
            if let Some(&idx) = self.node_index.get(&id) {
                let node = &self.nodes[idx];
                chain.push(node.entry.clone());
                cursor = node.parent_id.clone();
            } else {
                break;
            }
        }
        chain.reverse();
        chain
    }

    /// Produces a compacted view of up to `keep_tail` recent entries WITHOUT EVER
    /// splitting a `ToolCall` from its matching `ToolResult` entries.
    #[must_use]
    pub fn compact_for_request(&self, keep_tail: usize) -> Vec<SessionEntry> {
        let entries = self.active_entries();
        if entries.len() <= keep_tail || keep_tail == 0 {
            return entries;
        }

        let mut split_idx = entries.len().saturating_sub(keep_tail);

        // Advance or rewind split boundary so we NEVER land between ToolCall and ToolResult(s).
        while split_idx > 0 && is_tool_boundary_unsafe(&entries, split_idx) {
            split_idx -= 1;
        }

        if split_idx == 0 {
            return entries;
        }

        let compacted_summary = SessionEntry::CompactionSummary {
            compacted_nodes: split_idx,
            summary: format!(
                "Compacted {split_idx} earlier turns while preserving tool boundaries"
            ),
        };

        let mut result = Vec::with_capacity(1 + entries.len() - split_idx);
        result.push(compacted_summary);
        result.extend_from_slice(&entries[split_idx..]);
        result
    }

    /// Captures a reversible state/file checkpoint and records its reference on the active branch.
    pub fn record_checkpoint(
        &mut self,
        label: impl Into<String>,
        snapshots: BTreeMap<String, Option<Vec<u8>>>,
    ) -> String {
        let label = label.into();
        let checkpoint_id = uuid::Uuid::new_v4().to_string();
        let parent_node_id = self.active_head().map(ToOwned::to_owned);
        self.checkpoints.insert(
            checkpoint_id.clone(),
            FileCheckpoint {
                checkpoint_id: checkpoint_id.clone(),
                parent_node_id,
                label: label.clone(),
                snapshots,
                created_at_ms: now_ms(),
            },
        );
        self.append(SessionEntry::CheckpointRef {
            checkpoint_id: checkpoint_id.clone(),
            label,
        });
        checkpoint_id
    }

    #[must_use]
    pub fn get_checkpoint(&self, checkpoint_id: &str) -> Option<&FileCheckpoint> {
        self.checkpoints.get(checkpoint_id)
    }

    /// Records a memory or instinct note with explicit source provenance.
    pub fn add_memory(
        &mut self,
        content: impl Into<String>,
        provenance_source: impl Into<String>,
        instinct_tag: Option<String>,
        confidence_pct: u8,
    ) -> MemoryEntry {
        let item = MemoryEntry {
            id: uuid::Uuid::new_v4().to_string(),
            content: content.into(),
            provenance_source: provenance_source.into(),
            instinct_tag,
            confidence_pct: confidence_pct.min(100),
            recorded_at_ms: now_ms(),
        };
        self.memories.push(item.clone());
        item
    }

    #[must_use]
    pub fn memories(&self) -> &[MemoryEntry] {
        &self.memories
    }
}

fn is_tool_boundary_unsafe(entries: &[SessionEntry], split_idx: usize) -> bool {
    // Unsafe if the tail starts directly with a ToolResult (its ToolCall was in the compacted prefix)
    // or if the prefix ends with a ToolCall without its following ToolResult.
    matches!(
        entries.get(split_idx),
        Some(SessionEntry::ToolResult { .. })
    ) || split_idx
        .checked_sub(1)
        .and_then(|prev| entries.get(prev))
        .is_some_and(|e| matches!(e, SessionEntry::ToolCall { .. }))
}
