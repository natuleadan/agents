//! Ultra-lean native Rust two-level agent execution engine (`agents-runtime`),
//! branching session tree with atomic tool compaction, host hooks, and multi-provider adapters.

mod batch;
pub mod engine;
pub mod hooks;
pub mod providers;
pub mod session_tree;

pub use engine::{
    NativeAgentRuntime, NativeRuntimeConfig, NativeTurnOutcome, RetryAuditEntry, RuntimeTool,
    ToolCallRequest, ToolExecutionResult,
};
pub use hooks::{DefaultLoopHooks, LoopHooks, OutputSchemaContract, RetryPolicy};
pub use providers::{
    HttpProviderClient, ModelClient, ModelTurnRequest, ModelTurnResponse, ProviderEndpointConfig,
    ProviderKind, ProviderRegistry, ScriptedModelClient,
};
pub use session_tree::{
    FileCheckpoint, MemoryEntry, SessionBranch, SessionEntry, SessionNode, SessionTree,
};
