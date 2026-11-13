//! Hexagonal ports and adapters tests for Phase 11 third-party cloud integration.

use agents_server::infra::adapters::{
    BlobStorePort, InfrastructureRegistry, LocalFsBlobStore, QstashMessageBus, VectorMatch,
    VectorStorePort, vector::InMemoryVectorStore,
};
use agents_server::infra::config::{ConfigError, InfrastructureMode, ServerConfig};
use sha2::{Digest, Sha256};

fn compute_hmac_hex(key: &[u8], msg: &[u8]) -> String {
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
    let inner_hash = inner.finalize();

    let mut outer = Sha256::new();
    outer.update(o_pad);
    outer.update(inner_hash);
    hex::encode(outer.finalize())
}

#[tokio::test]
async fn in_memory_vector_store_indexes_and_queries_by_cosine_similarity() {
    let store = InMemoryVectorStore::new();

    let v1 = vec![1.0, 0.0, 0.0];
    let v2 = vec![0.0, 1.0, 0.0];
    let v3 = vec![
        std::f32::consts::FRAC_1_SQRT_2,
        std::f32::consts::FRAC_1_SQRT_2,
        0.0,
    ];

    store
        .upsert("doc-1", &v1, Some(serde_json::json!({ "tag": "x-axis" })))
        .await
        .expect("upsert doc-1");
    store
        .upsert("doc-2", &v2, Some(serde_json::json!({ "tag": "y-axis" })))
        .await
        .expect("upsert doc-2");
    store
        .upsert("doc-3", &v3, Some(serde_json::json!({ "tag": "diagonal" })))
        .await
        .expect("upsert doc-3");

    // Query close to v1
    let query_v1 = vec![0.95, 0.05, 0.0];
    let matches: Vec<VectorMatch> = store.query(&query_v1, 2).await.expect("query matches");

    assert_eq!(matches.len(), 2);
    assert_eq!(matches[0].id, "doc-1");
    assert!(matches[0].score > 0.99);

    // Delete doc-1 and query again
    store.delete("doc-1").await.expect("delete doc-1");
    let after_del = store.query(&query_v1, 2).await.expect("query after delete");
    assert_eq!(after_del[0].id, "doc-3");
}

#[tokio::test]
async fn local_fs_blob_store_puts_gets_and_blocks_path_traversal() {
    let temp_dir = std::env::temp_dir().join(format!("agents-blob-test-{}", uuid::Uuid::new_v4()));
    let store = LocalFsBlobStore::new(temp_dir.clone());

    let data = b"hello artifact blob data".to_vec();
    let key = "reports/summary.md";

    let stored_key = store
        .put_blob(key, data.clone(), "text/markdown")
        .await
        .expect("put blob");
    assert_eq!(stored_key, key);

    let retrieved = store.get_blob(key).await.expect("get blob");
    assert_eq!(retrieved, data);

    // Path traversal attempt fails closed
    let traversal_res = store.get_blob("../etc/passwd").await;
    assert!(traversal_res.is_err());

    store.delete_blob(key).await.expect("delete blob");
    let after_del = store.get_blob(key).await;
    assert!(after_del.is_err());

    let _ = tokio::fs::remove_dir_all(temp_dir).await;
}

#[test]
fn qstash_webhook_signature_verification_enforces_hmac_and_rotation() {
    let key1 = "sigkey_current_test_1234567890abcdef";
    let key2 = "sigkey_next_test_0987654321fedcba";
    let payload = b"{\"event\":\"task.completed\",\"taskId\":\"task-42\"}";

    let valid_sig1 = compute_hmac_hex(key1.as_bytes(), payload);
    let valid_sig2 = compute_hmac_hex(key2.as_bytes(), payload);

    // Both current and rotated next keys verify successfully
    assert!(QstashMessageBus::verify_signature(
        payload,
        &valid_sig1,
        Some(key1),
        Some(key2)
    ));
    assert!(QstashMessageBus::verify_signature(
        payload,
        &valid_sig2,
        Some(key1),
        Some(key2)
    ));

    // Tampered payload fails
    let tampered_payload = b"{\"event\":\"task.tampered\",\"taskId\":\"task-42\"}";
    assert!(!QstashMessageBus::verify_signature(
        tampered_payload,
        &valid_sig1,
        Some(key1),
        Some(key2)
    ));

    // Tampered signature fails
    assert!(!QstashMessageBus::verify_signature(
        payload,
        "00112233445566778899aabbccddeeff00112233445566778899aabbccddeeff",
        Some(key1),
        Some(key2)
    ));
}

#[tokio::test]
async fn infrastructure_registry_bootstraps_ports_by_mode() {
    // Self-hosted in-memory registry
    let mem_reg = InfrastructureRegistry::in_memory();
    assert_eq!(mem_reg.mode, InfrastructureMode::SelfHosted);

    // Managed cloud config missing required keys fails closed
    let cloud_config = ServerConfig::from_lookup(|k| match k {
        "INFRASTRUCTURE_MODE" => Some("managed-cloud".to_owned()),
        "DATABASE_URL" => Some("postgres://user:pass@ep-test.neon.tech/db".to_owned()),
        "REDIS_URL" => Some("rediss://default:token@accepted-skunk.upstash.io:6379".to_owned()),
        _ => None,
    });
    assert!(matches!(
        cloud_config,
        Err(ConfigError::MissingCloudRequirement("QSTASH_URL"))
    ));

    // Valid cloud config succeeds
    let valid_cloud_config = ServerConfig::from_lookup(|k| match k {
        "INFRASTRUCTURE_MODE" => Some("managed-cloud".to_owned()),
        "DATABASE_URL" => Some("postgres://user:pass@ep-test.neon.tech/db".to_owned()),
        "REDIS_URL" => Some("rediss://default:token@accepted-skunk.upstash.io:6379".to_owned()),
        "QSTASH_URL" => Some("https://qstash.upstash.io".to_owned()),
        _ => None,
    })
    .expect("valid cloud config");

    assert_eq!(
        valid_cloud_config.infrastructure_mode,
        InfrastructureMode::ManagedCloud
    );
}
