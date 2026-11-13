//! Integration test verifying preflight doctor diagnostics across Topología A and Topología B.

#![forbid(unsafe_code)]

use agents_server::{CheckStatus, DoctorReport, EnvironmentMode, InfrastructureMode, ServerConfig};

#[tokio::test]
async fn doctor_evaluates_topologia_a_self_hosted_dev_cleanly() {
    let config = ServerConfig::from_lookup(|k| match k {
        "AGENTS_ENV" => Some("development".to_owned()),
        "INFRASTRUCTURE_MODE" => Some("self-hosted".to_owned()),
        _ => None,
    })
    .expect("valid dev config");

    let report = DoctorReport::run(&config).await;
    assert_eq!(report.mode, InfrastructureMode::SelfHosted);
    assert_eq!(report.env, EnvironmentMode::Development);
    assert!(
        report.healthy,
        "dev self-hosted fallback should report healthy"
    );

    // Must contain entries for Database, State, Bus, Sandboxes, Providers, Auxiliary
    let names: Vec<&str> = report.items.iter().map(|i| i.name.as_str()).collect();
    assert!(names.iter().any(|n| n.contains("Database")));
    assert!(names.iter().any(|n| n.contains("State")));
    assert!(names.iter().any(|n| n.contains("Message Bus")));
    assert!(names.iter().any(|n| n.contains("Sandbox")));
}

#[tokio::test]
async fn doctor_evaluates_topologia_b_cloud_native_preflight() {
    let config = ServerConfig::from_lookup(|k| match k {
        "AGENTS_ENV" => Some("test".to_owned()),
        "INFRASTRUCTURE_MODE" => Some("managed-cloud".to_owned()),
        "DATABASE_URL" => Some("postgres://user:pass@ep-test.neon.tech/agents".to_owned()),
        "REDIS_URL" => Some("rediss://default:token@skunk.upstash.io:6379".to_owned()),
        "QSTASH_URL" => Some("https://qstash.upstash.io".to_owned()),
        "QSTASH_TOKEN" => Some("ey-test-token".to_owned()),
        "UPSTASH_BOX_TOKEN" => Some("box-test-token".to_owned()),
        "DEEPINFRA_TOKEN" => Some("deepinfra-key-12345".to_owned()),
        _ => None,
    })
    .expect("valid cloud config");

    let report = DoctorReport::run(&config).await;
    assert_eq!(report.mode, InfrastructureMode::ManagedCloud);

    let bus_item = report
        .items
        .iter()
        .find(|i| i.name.contains("QStash"))
        .expect("QStash item present");
    assert_eq!(bus_item.status, CheckStatus::Pass);

    let sandbox_item = report
        .items
        .iter()
        .find(|i| i.name.contains("Sandbox"))
        .expect("Sandbox item present");
    assert_eq!(sandbox_item.status, CheckStatus::Pass);

    let inference_item = report
        .items
        .iter()
        .find(|i| i.name.contains("Inference"))
        .expect("Inference item present");
    assert_eq!(inference_item.status, CheckStatus::Pass);
    assert!(inference_item.message.contains("DeepInfra"));
}

#[tokio::test]
async fn doctor_fails_when_cloud_credentials_are_missing() {
    let config = ServerConfig::for_tests();
    // Force managed-cloud mode with missing cloud credentials
    let mut cloud_config = config;
    cloud_config.infrastructure_mode = InfrastructureMode::ManagedCloud;
    cloud_config.upstash_redis_url = None;
    cloud_config.qstash_url = None;
    cloud_config.upstash_box_token = None;
    cloud_config.deepinfra_token = None;

    let report = DoctorReport::run(&cloud_config).await;
    assert!(
        !report.healthy,
        "Report should fail with missing cloud keys"
    );

    let state_check = report
        .items
        .iter()
        .find(|i| i.name.contains("State"))
        .expect("State item present");
    assert_eq!(state_check.status, CheckStatus::Fail);

    let bus_check = report
        .items
        .iter()
        .find(|i| i.name.contains("QStash"))
        .expect("QStash item present");
    assert_eq!(bus_check.status, CheckStatus::Fail);
}
