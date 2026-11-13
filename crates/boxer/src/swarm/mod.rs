//! Swarm coordination trust boundaries on Boxer nodes:
//! OWASP ASI-05 HMAC-SHA256 message signing/verification, OWASP ASI-07 permission narrowing,
//! and worktree-per-task workspace isolation.

use std::path::{Path, PathBuf};

use agents_protocol::{AgentHandoff, PermissionMask, SwarmMessage, SwarmTopic};
use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;

use crate::{
    config::now_ms,
    error::BoxerError,
    leases::{CleanupDropGuard, SandboxResources},
    sandbox::policy::resolve_confined_workdir,
};

/// Computes deterministic NATS subject for a [`SwarmMessage`] topic.
#[must_use]
pub fn swarm_nats_subject(session_id: &str, topic: &SwarmTopic) -> String {
    match topic {
        SwarmTopic::Broadcast => format!("swarm.{session_id}.broadcast"),
        SwarmTopic::DirectAgent(agent_id) => format!("swarm.{session_id}.agent.{agent_id}"),
    }
}

/// Computes HMAC-SHA256 hex signature covering canonical swarm message envelope fields
/// to enforce OWASP ASI-05 message integrity across NATS.
#[must_use]
pub fn compute_swarm_signature(secret: &[u8], msg: &SwarmMessage) -> String {
    let payload_canon = serde_json::to_string(&msg.payload).unwrap_or_default();
    let perms_canon = serde_json::to_string(&msg.permission_mask).unwrap_or_default();
    let topic_label = swarm_nats_subject(&msg.session_id, &msg.to);
    let canonical = format!(
        "{}|{}|{}|{}|{}|{}|{}",
        msg.message_id,
        msg.session_id,
        msg.from_agent_id,
        topic_label,
        payload_canon,
        perms_canon,
        msg.emitted_at_ms
    );
    hmac_sha256_hex(secret, canonical.as_bytes())
}

/// Creates a signed [`SwarmMessage`].
#[must_use]
pub fn build_signed_swarm_message(
    secret: &[u8],
    session_id: impl Into<String>,
    from_agent_id: impl Into<String>,
    to: SwarmTopic,
    payload: serde_json::Value,
    permission_mask: PermissionMask,
) -> SwarmMessage {
    let mut msg = SwarmMessage {
        message_id: uuid::Uuid::new_v4().to_string(),
        session_id: session_id.into(),
        from_agent_id: from_agent_id.into(),
        to,
        payload,
        permission_mask,
        signature: String::new(),
        emitted_at_ms: now_ms(),
    };
    msg.signature = compute_swarm_signature(secret, &msg);
    msg
}

/// Verifies constant-time HMAC-SHA256 integrity on an incoming [`SwarmMessage`].
///
/// # Errors
///
/// Returns [`BoxerError::IsolationViolation`] if signature is missing or invalid.
pub fn verify_swarm_signature(secret: &[u8], msg: &SwarmMessage) -> Result<(), BoxerError> {
    if msg.signature.is_empty() {
        return Err(BoxerError::IsolationViolation(
            "unsigned swarm message rejected (OWASP ASI-05)".into(),
        ));
    }
    let expected = compute_swarm_signature(secret, msg);
    if expected
        .as_bytes()
        .ct_eq(msg.signature.as_bytes())
        .unwrap_u8()
        != 1
    {
        return Err(BoxerError::IsolationViolation(
            "swarm message signature verification failed (OWASP ASI-05)".into(),
        ));
    }
    Ok(())
}

/// Enforces OWASP ASI-07 strict permission subset narrowing before subagent delegation.
///
/// # Errors
///
/// Returns [`BoxerError::Permission`] when child requests escalated capabilities or tools.
pub fn validate_child_handoff(
    parent_mask: &PermissionMask,
    parent_agent_id: impl Into<String>,
    child_agent_id: impl Into<String>,
    requested_mask: &PermissionMask,
    worktree_branch: Option<String>,
    goal: impl Into<String>,
) -> Result<AgentHandoff, BoxerError> {
    let delegated_mask = parent_mask.narrow(requested_mask)?;
    Ok(AgentHandoff {
        handoff_id: uuid::Uuid::new_v4().to_string(),
        parent_agent_id: parent_agent_id.into(),
        child_agent_id: child_agent_id.into(),
        worktree_branch,
        delegated_mask,
        goal: goal.into(),
    })
}

/// Drop-guarded lease handle bound to an isolated task worktree directory.
pub type WorktreeGuard = CleanupDropGuard<SandboxResources, fn(SandboxResources)>;

/// Provisions an isolated worktree directory scoped to `session_id/worktrees/<task_id>`
/// and returns a `DropGuard` for deterministic cleanup.
///
/// # Errors
///
/// Returns [`BoxerError::IsolationViolation`] if `task_id` attempts path traversal.
pub fn provision_task_worktree(
    workspace_root: &Path,
    session_id: &str,
    task_id: &str,
) -> Result<(PathBuf, WorktreeGuard), BoxerError> {
    let safe_task: String = task_id
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
        .collect();
    if safe_task.is_empty() {
        return Err(BoxerError::IsolationViolation(
            "task worktree id must be alphanumeric".into(),
        ));
    }
    let rel = format!("worktrees/{safe_task}");
    let dir = resolve_confined_workdir(workspace_root, session_id, Some(&rel))?;
    let mut res = SandboxResources::new(task_id);
    res.preview_route_file = None;
    fn noop_cleanup(res: SandboxResources) {
        res.cleanup_sync();
    }
    Ok((dir, CleanupDropGuard::new(res, noop_cleanup)))
}

fn hmac_sha256_hex(key: &[u8], msg: &[u8]) -> String {
    const BLOCK_SIZE: usize = 64;
    let mut k_block = [0u8; BLOCK_SIZE];
    if key.len() > BLOCK_SIZE {
        let hashed = Sha256::digest(key);
        k_block[..32].copy_from_slice(&hashed);
    } else {
        k_block[..key.len()].copy_from_slice(key);
    }

    let mut o_pad = [0x5cu8; BLOCK_SIZE];
    let mut i_pad = [0x36u8; BLOCK_SIZE];
    for i in 0..BLOCK_SIZE {
        o_pad[i] ^= k_block[i];
        i_pad[i] ^= k_block[i];
    }

    let mut inner = Sha256::new();
    inner.update(i_pad);
    inner.update(msg);
    let inner_digest = inner.finalize();

    let mut outer = Sha256::new();
    outer.update(o_pad);
    outer.update(inner_digest);
    hex::encode(outer.finalize())
}
