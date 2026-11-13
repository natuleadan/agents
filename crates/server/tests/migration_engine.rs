//! Integration tests for migration snapshot export, import validation, and tamper detection.

#![forbid(unsafe_code)]

use agents_server::{
    MigrationSnapshot,
    infra::migrate::{
        export::validate_in_memory_audit_chain,
        types::{
            MigrateApiKeyRecord, MigrateAutocronRecord, MigrateRenderedSlugRecord,
            MigrateSessionRecord, MigrateUserRecord, MigrateWorkflowRecord,
        },
    },
    modules::audit::{GENESIS_AUDIT_HASH, compute_entry_hash, entities::AuditLogRecord},
};
use serde_json::json;

#[test]
fn in_memory_audit_chain_validates_genesis_and_sequential_entries() {
    let mut logs = Vec::new();

    // Entry 1
    let id_1 = "audit-001".to_owned();
    let prev_1 = GENESIS_AUDIT_HASH.to_owned();
    let details_1 = json!({"ip": "127.0.0.1"});
    let entry_hash_1 = compute_entry_hash(
        &prev_1,
        &id_1,
        Some("usr-admin"),
        "auth.login",
        "user",
        Some("usr-admin"),
        "success",
        &details_1,
        1000,
    );
    logs.push(AuditLogRecord {
        id: id_1.clone(),
        actor_id: Some("usr-admin".to_owned()),
        action: "auth.login".to_owned(),
        resource_type: "user".to_owned(),
        resource_id: Some("usr-admin".to_owned()),
        outcome: "success".to_owned(),
        details: details_1,
        prev_hash: prev_1,
        entry_hash: entry_hash_1.clone(),
        created_at: 1000,
    });

    // Entry 2
    let id_2 = "audit-002".to_owned();
    let prev_2 = entry_hash_1;
    let details_2 = json!({"workflow": "wf-build"});
    let entry_hash_2 = compute_entry_hash(
        &prev_2,
        &id_2,
        Some("usr-admin"),
        "workflow.trigger",
        "workflow",
        Some("wf-build"),
        "success",
        &details_2,
        2000,
    );
    logs.push(AuditLogRecord {
        id: id_2,
        actor_id: Some("usr-admin".to_owned()),
        action: "workflow.trigger".to_owned(),
        resource_type: "workflow".to_owned(),
        resource_id: Some("wf-build".to_owned()),
        outcome: "success".to_owned(),
        details: details_2,
        prev_hash: prev_2,
        entry_hash: entry_hash_2,
        created_at: 2000,
    });

    let (valid, head_hash) = validate_in_memory_audit_chain(&logs);
    assert!(valid, "Valid chained audit logs must pass verification");
    assert!(!head_hash.is_empty());
}

#[test]
fn in_memory_audit_chain_detects_tampering_or_broken_links() {
    let mut logs = Vec::new();
    let id_1 = "audit-001".to_owned();
    let prev_1 = GENESIS_AUDIT_HASH.to_owned();
    let details_1 = json!({"ip": "127.0.0.1"});
    let entry_hash_1 = compute_entry_hash(
        &prev_1,
        &id_1,
        Some("usr-admin"),
        "auth.login",
        "user",
        Some("usr-admin"),
        "success",
        &details_1,
        1000,
    );
    logs.push(AuditLogRecord {
        id: id_1,
        actor_id: Some("usr-admin".to_owned()),
        action: "auth.login".to_owned(),
        resource_type: "user".to_owned(),
        resource_id: Some("usr-admin".to_owned()),
        outcome: "success".to_owned(),
        details: details_1,
        prev_hash: prev_1,
        entry_hash: entry_hash_1,
        created_at: 1000,
    });

    // Tampered entry with invalid prev_hash
    logs.push(AuditLogRecord {
        id: "audit-002".to_owned(),
        actor_id: Some("attacker".to_owned()),
        action: "task.execute".to_owned(),
        resource_type: "task".to_owned(),
        resource_id: Some("tsk-001".to_owned()),
        outcome: "success".to_owned(),
        details: json!({}),
        prev_hash: "deadbeefcafebabe".to_owned(),
        entry_hash: "invalidhash".to_owned(),
        created_at: 2000,
    });

    let (valid, _head) = validate_in_memory_audit_chain(&logs);
    assert!(!valid, "Tampered chain must be detected and rejected");
}

#[test]
fn migration_snapshot_json_roundtrip_preserves_all_entities() {
    let snapshot = MigrationSnapshot {
        version: 1,
        exported_at: 1_700_000_000,
        source_mode: "SelfHosted".to_owned(),
        users: vec![MigrateUserRecord {
            id: "usr-01".to_owned(),
            email: "admin@example.com".to_owned(),
            password_hash: "$argon2id$...".to_owned(),
            role: "admin".to_owned(),
            created_at: 100,
        }],
        api_keys: vec![MigrateApiKeyRecord {
            id: "key-01".to_owned(),
            user_id: "usr-01".to_owned(),
            name: "cli-token".to_owned(),
            key_hash: "hash01".to_owned(),
            scopes: json!(["read", "write"]),
            expires_at: None,
            created_at: 110,
        }],
        sessions: vec![MigrateSessionRecord {
            id: "sess-01".to_owned(),
            user_id: "usr-01".to_owned(),
            title: "Analysis Session".to_owned(),
            metadata: json!({"tier": "standard"}),
            created_at: 120,
            updated_at: 130,
        }],
        workflows: vec![MigrateWorkflowRecord {
            id: "wf-01".to_owned(),
            creator_id: "usr-01".to_owned(),
            name: "Data Ingestion".to_owned(),
            description: "ETL flow".to_owned(),
            dag: json!({"nodes": []}),
            is_active: true,
            approval_role: None,
            approved_at: None,
            approved_by: None,
            created_at: 140,
            updated_at: 150,
        }],
        autocron_schedules: vec![MigrateAutocronRecord {
            id: "cron-01".to_owned(),
            workflow_id: "wf-01".to_owned(),
            schedule_cron: "0 * * * *".to_owned(),
            description: "Hourly sync".to_owned(),
            is_active: true,
            max_jitter_secs: 30,
            created_at: 160,
            updated_at: 170,
        }],
        rendered_slugs: vec![MigrateRenderedSlugRecord {
            slug: "daily-report".to_owned(),
            workflow_id: "wf-01".to_owned(),
            run_id: "run-01".to_owned(),
            content_type: "text/html".to_owned(),
            rendered_body: "<h1>Report</h1>".to_owned(),
            published_by: "usr-01".to_owned(),
            published_at: 180,
            expires_at: None,
        }],
        audit_logs: Vec::new(),
        audit_hash_chain_valid: true,
        audit_head_hash: GENESIS_AUDIT_HASH.to_owned(),
    };

    let serialized = serde_json::to_string(&snapshot).expect("serialize snapshot");
    let deserialized: MigrationSnapshot =
        serde_json::from_str(&serialized).expect("deserialize snapshot");

    assert_eq!(deserialized.version, 1);
    assert_eq!(deserialized.users.len(), 1);
    assert_eq!(deserialized.api_keys.len(), 1);
    assert_eq!(deserialized.sessions.len(), 1);
    assert_eq!(deserialized.workflows.len(), 1);
    assert_eq!(deserialized.autocron_schedules.len(), 1);
    assert_eq!(deserialized.rendered_slugs.len(), 1);
    assert_eq!(deserialized.rendered_slugs[0].slug, "daily-report");
}
