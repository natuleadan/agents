#![allow(clippy::unwrap_used, clippy::expect_used)]

use agents_boxer::{
    runtime::{DurableReplayTracker, DurableStepLogger, InMemoryDurableLogger},
    swarm::{build_signed_swarm_message, validate_child_handoff, verify_swarm_signature},
};
use agents_protocol::{PermissionMask, SwarmTopic};
use serde_json::json;

fn parent_permission_mask() -> PermissionMask {
    PermissionMask {
        can_read_fs: true,
        can_write_fs: true,
        can_exec_shell: false,
        can_network: false,
        allowed_tools: vec!["read_file".to_owned(), "format_poem".to_owned()],
    }
}

#[tokio::test]
async fn pi_durable_replay_tracker_skips_duplicate_tool_execution() {
    let logger = InMemoryDurableLogger::new();
    let mut tracker = DurableReplayTracker::new("task-crash-1", "session-crash-1");

    // Execute first tool call and record it
    let step_0 = tracker.build_tool_step(
        "call_file_read_1",
        "read_file",
        json!({ "path": "/workspace/poem.txt" }),
        json!({ "content": "The moon shines bright across the sea." }),
        false,
    );
    logger.append_step(step_0.clone()).await.unwrap();

    // Checkpoint progress
    let step_1 = tracker.build_checkpoint_step(json!({
        "completedSteps": ["step_0"],
        "linesRead": 1
    }));
    logger.append_step(step_1.clone()).await.unwrap();

    // Verify logger stored steps
    let stored = logger.list_steps("task-crash-1").await.unwrap();
    assert_eq!(stored.len(), 2);

    // SIMULATE CRASH & RESTART:
    // Create brand new tracker instance and recover from stored append-only logs
    let mut resumed_tracker = DurableReplayTracker::new("task-crash-1", "session-crash-1");
    resumed_tracker.recover_from_steps(&stored);

    // Verify crash recovery state
    assert!(resumed_tracker.has_tool_completed("call_file_read_1"));
    let cached = resumed_tracker
        .get_cached_tool_result("call_file_read_1")
        .expect("cached result must exist");
    assert_eq!(
        cached.output_payload,
        Some(json!({ "content": "The moon shines bright across the sea." }))
    );

    // Verify last checkpoint recovered
    let cp = resumed_tracker
        .last_checkpoint()
        .expect("checkpoint must exist");
    assert_eq!(cp["linesRead"], 1);

    // Non-executed call_id must NOT be marked completed
    assert!(!resumed_tracker.has_tool_completed("call_file_write_2"));
}

#[test]
fn inter_container_swarm_peering_hmac_and_permission_narrowing() {
    let secret = b"super-secure-shared-swarm-cluster-key-32b!";
    let parent_mask = parent_permission_mask();

    // 1. Build signed swarm message
    let signed_msg = build_signed_swarm_message(
        secret,
        "session-cluster-1",
        "agent-leader",
        SwarmTopic::DirectAgent("agent-worker".to_owned()),
        json!({ "task": "format_poem", "verse": 1 }),
        parent_mask.clone(),
    );

    // 2. Verify signature on worker container
    assert!(
        verify_swarm_signature(secret, &signed_msg).is_ok(),
        "valid HMAC-SHA256 signature must be verified"
    );

    // 3. Reject tampered message (OWASP ASI-05)
    let mut tampered_msg = signed_msg.clone();
    tampered_msg.payload = json!({ "task": "steal_secrets" });
    assert!(
        verify_swarm_signature(secret, &tampered_msg).is_err(),
        "tampered payload must fail verification closed"
    );

    // 4. Reject message with invalid secret
    assert!(
        verify_swarm_signature(b"wrong-untrusted-secret-key!", &signed_msg).is_err(),
        "untrusted key must be rejected"
    );

    // 5. Enforce capability narrowing on subagent handoff (OWASP ASI-07)
    let mut escalated_mask = parent_mask.clone();
    escalated_mask.can_network = true; // Attempt to escalate network access!

    let handoff_res = validate_child_handoff(
        &parent_mask,
        "agent-leader",
        "agent-child",
        &escalated_mask,
        None,
        "escalate permissions",
    );
    assert!(
        handoff_res.is_err(),
        "escalation above parent permissions must fail closed (OWASP ASI-07)"
    );

    // Narrowed permission succeeds
    let mut narrowed_mask = parent_mask.clone();
    narrowed_mask.can_write_fs = false; // Further restricted
    let valid_handoff = validate_child_handoff(
        &parent_mask,
        "agent-leader",
        "agent-child",
        &narrowed_mask,
        None,
        "restricted worker",
    );
    assert!(valid_handoff.is_ok());
}
