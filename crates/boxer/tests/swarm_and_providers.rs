use std::path::PathBuf;

use agents_boxer::{
    error::BoxerError,
    runtime::{ProviderEndpointConfig, ProviderKind, ProviderRegistry},
    swarm::{
        build_signed_swarm_message, compute_swarm_signature, provision_task_worktree,
        swarm_nats_subject, validate_child_handoff, verify_swarm_signature,
    },
};
use agents_protocol::{PermissionMask, SwarmTopic};
use serde_json::json;

type TestResult = Result<(), Box<dyn std::error::Error>>;

fn read_mask() -> PermissionMask {
    PermissionMask {
        can_read_fs: true,
        can_write_fs: false,
        can_exec_shell: false,
        can_network: false,
        allowed_tools: vec!["grep".to_owned(), "read".to_owned()],
    }
}

fn unique_temp_dir(prefix: &str) -> Result<PathBuf, Box<dyn std::error::Error>> {
    let dir = std::env::temp_dir().join(format!("{prefix}-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir)?;
    Ok(dir)
}

#[test]
fn provider_registry_and_swarm_asi05_asi07_and_worktree_drop_guard() -> TestResult {
    let mut reg = ProviderRegistry::new();
    for (id, kind) in [
        ("ollama-local", ProviderKind::Ollama),
        ("lmstudio-local", ProviderKind::LmStudio),
        ("openai-main", ProviderKind::OpenAi),
        ("anthropic-main", ProviderKind::Anthropic),
        ("deepinfra-main", ProviderKind::DeepInfra),
        (
            "provider_endpoint:vllm-custom-99",
            ProviderKind::Custom("vllm".to_owned()),
        ),
    ] {
        reg.register(ProviderEndpointConfig {
            id: id.to_owned(),
            kind,
            base_url: "http://127.0.0.1:8080".to_owned(),
            model: "test-model".to_owned(),
            api_key_env: None,
            default_headers: Default::default(),
        });
    }
    assert!(reg.resolve("provider_endpoint:vllm-custom-99").is_some());
    assert!(reg.resolve("vllm-custom-99").is_some());

    // Swarm ASI-05 signed messages & NATS subjects
    let secret = b"phase4-swarm-hmac-secret";
    let mut msg = build_signed_swarm_message(
        secret,
        "sess-sw",
        "planner-1",
        SwarmTopic::DirectAgent("coder-2".to_owned()),
        json!({ "task": "implement module" }),
        read_mask(),
    );
    assert_eq!(
        swarm_nats_subject(&msg.session_id, &msg.to),
        "swarm.sess-sw.agent.coder-2"
    );
    assert_eq!(msg.signature, compute_swarm_signature(secret, &msg));
    verify_swarm_signature(secret, &msg)?;

    // Tampering fails closed
    msg.payload = json!({ "task": "rm -rf /" });
    assert!(verify_swarm_signature(secret, &msg).is_err());

    // Swarm ASI-07 child permission escalation rejection
    let parent = read_mask();
    let mut escalated = read_mask();
    escalated.can_exec_shell = true;
    let err = validate_child_handoff(
        &parent,
        "planner-1",
        "coder-2",
        &escalated,
        Some("feat/sub".to_owned()),
        "try shell",
    );
    assert!(matches!(err, Err(BoxerError::Permission(_))));

    let valid = validate_child_handoff(
        &parent,
        "planner-1",
        "reader-2",
        &read_mask(),
        Some("feat/sub".to_owned()),
        "inspect code",
    )?;
    assert_eq!(valid.delegated_mask, parent);

    // Worktree isolation + DropGuard
    let root = unique_temp_dir("boxer-worktree")?;
    let canonical_root = std::fs::canonicalize(&root)?;
    let (wt_path, _guard) = provision_task_worktree(&root, "sess-wt", "task_alpha_1")?;
    assert!(wt_path.starts_with(&canonical_root));
    assert!(provision_task_worktree(&root, "sess-wt", "../etc").is_ok());
    assert!(provision_task_worktree(&root, "sess-wt", "///").is_err());
    let _ = std::fs::remove_dir_all(&root);
    Ok(())
}
