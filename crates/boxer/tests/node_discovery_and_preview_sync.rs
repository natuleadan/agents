use std::{path::PathBuf, sync::Arc};

use agents_boxer::{
    config::BoxerConfig,
    daemon::{BoxerDaemon, InMemoryBoxerBus},
    leases::InMemoryLeaseClient,
};
use agents_protocol::{
    IsolationKind, NodeStatus, PreviewRouteSync, RuntimeKind, SandboxProfile, SetupMode,
    TraefikPreviewRoute,
};

type TestResult = Result<(), Box<dyn std::error::Error>>;

fn test_config() -> Result<BoxerConfig, Box<dyn std::error::Error>> {
    Ok(BoxerConfig {
        node_id: "boxer-vlan-01".to_owned(),
        default_profile: SandboxProfile {
            mode: SetupMode::Local,
            runtime: RuntimeKind::Docker,
            isolation: IsolationKind::Openshell,
            allow_host_native: false,
            gateway_url: Some("http://10.0.0.10:9443".to_owned()),
        },
        sandbox_image: "ghcr.io/natuleadan/agents-sandbox:latest".to_owned(),
        workspace_root: PathBuf::from("/tmp/agents-boxes-test"),
        max_concurrent_boxes: 6,
        nats_url: "nats://10.0.0.10:4222".to_owned(),
        nats_tls_enabled: false,
        nats_nkey_seed: None,
        nats_jwt: None,
        server_internal_url: "http://10.0.0.10:8080".to_owned(),
        worker_token: "worker-test-token".to_owned(),
        lease_ttl_seconds: 120,
        lease_heartbeat_seconds: 20,
        proxy_bind_addr: "127.0.0.1:0".parse()?,
        private_vlan_only: true,
        proxy_allowed_hosts: Vec::new(),
        proxy_upstream_headers: Default::default(),
        traefik_preview_enabled: true,
        traefik_preview_domain: "preview.example.com".to_owned(),
        traefik_dynamic_dir: std::env::temp_dir()
            .join(format!("agents-traefik-node-test-{}", uuid::Uuid::new_v4())),
        traefik_default_ttl_secs: 600,
        strict_native_isolation: true,
        upstash_box_token: None,
        deepinfra_token: None,
        http_port: 9091,
        cluster_secret: "dev-boxer-cluster-secret".to_owned(),
    })
}

#[tokio::test]
async fn boxer_announces_vlan_node_and_publishes_distributed_preview_sync() -> TestResult {
    let bus = InMemoryBoxerBus::new();
    let leases = InMemoryLeaseClient::new();
    let daemon = BoxerDaemon::new(test_config()?, leases, Arc::clone(&bus) as _)?;

    let announced = daemon
        .announce_node("10.0.1.42", NodeStatus::Online)
        .await?;
    assert_eq!(announced.node_id, "boxer-vlan-01");
    assert_eq!(announced.vlan_ip, "10.0.1.42");
    assert_eq!(announced.max_concurrent_boxes, 6);

    let sync = PreviewRouteSync {
        route: TraefikPreviewRoute {
            route_id: "route-sync-1".to_owned(),
            task_id: "task-1".to_owned(),
            slug: "demo-burger".to_owned(),
            internal_host: "10.0.1.42".to_owned(),
            internal_port: 8081,
            public_url: "http://127.0.0.1/preview/demo-burger".to_owned(),
            ttl_seconds: 300,
            requires_token: false,
            created_at_ms: 1_700_000_000_000,
        },
        token_sha256: None,
        revoked: false,
    };
    daemon.sync_preview_route(&sync).await?;

    assert_eq!(bus.recorded_announcements().await.len(), 1);
    assert_eq!(bus.recorded_preview_syncs().await.len(), 1);
    Ok(())
}
