#![allow(clippy::unwrap_used)]

use agents_protocol::{
    API_GENERATION, ApiKeyScope, ArtifactEntry, ArtifactManifest, ArtifactValidationError,
    Capability, CodecId, EventEnvelope, HandshakeResponse, IsolationKind, LoginRequest,
    METHOD_CATALOG, PermissionError, PermissionMask, RefreshRequest, RequestId, RpcErrorCode,
    RpcRequest, RuntimeKind, SandboxProfile, SandboxProfileError, SetupMode, StopReason,
    StreamPayload, TaskRequest, TaskState, ThirdPartyProvider, TokenPair, TriStateFilter,
    TriggerKind, WorkerStatus, export_protocol_schema,
};
use serde::Deserialize;
use serde_json::{Value, json};

fn assert_frozen_roundtrip<T>(raw: &str)
where
    T: serde::Serialize + for<'de> serde::Deserialize<'de>,
{
    let expected: Value = serde_json::from_str(raw).unwrap();
    let parsed: T = serde_json::from_str(raw).unwrap();
    let serialized = serde_json::to_value(parsed).unwrap();
    assert_eq!(serialized, expected);
}

#[test]
fn frozen_fixtures_match_exact_wire_format() {
    assert_frozen_roundtrip::<HandshakeResponse>(include_str!("fixtures/handshake.v1.json"));
    assert_frozen_roundtrip::<TaskRequest>(include_str!("fixtures/task_request.v1.json"));
    assert_frozen_roundtrip::<EventEnvelope>(include_str!("fixtures/event_envelope.v1.json"));
    assert_frozen_roundtrip::<SandboxProfile>(include_str!("fixtures/sandbox_profile.v1.json"));
}

#[test]
fn handshake_compatibility_and_capabilities() {
    let resp: HandshakeResponse =
        serde_json::from_str(include_str!("fixtures/handshake.v1.json")).unwrap();
    assert!(resp.is_compatible_with(API_GENERATION));
    assert!(!resp.is_compatible_with(0));
    assert!(!resp.is_compatible_with(99));
    assert!(resp.supports(&Capability::TraefikPreview));
    assert!(!resp.supports(&Capability::Unknown("future_cap".to_owned())));
}

#[test]
fn unknown_enum_variants_preserve_forward_compatibility() {
    let codec: CodecId = serde_json::from_str("\"events.v9\"").unwrap();
    assert_eq!(codec, CodecId::Unknown("events.v9".to_owned()));
    assert_eq!(serde_json::to_string(&codec).unwrap(), "\"events.v9\"");

    let cap: Capability = serde_json::from_str("\"voice_v2\"").unwrap();
    assert_eq!(cap, Capability::Unknown("voice_v2".to_owned()));

    let scope: ApiKeyScope = serde_json::from_str("\"audit_read\"").unwrap();
    assert_eq!(scope, ApiKeyScope::Unknown("audit_read".to_owned()));

    let provider: ThirdPartyProvider = serde_json::from_str("\"future_box\"").unwrap();
    assert_eq!(
        provider,
        ThirdPartyProvider::Unknown("future_box".to_owned())
    );

    let iso: IsolationKind = serde_json::from_str("\"microvm\"").unwrap();
    assert_eq!(iso, IsolationKind::Unknown("microvm".to_owned()));

    let state: TaskState = serde_json::from_str("\"migrating\"").unwrap();
    assert_eq!(state, TaskState::Unknown("migrating".to_owned()));

    let worker: WorkerStatus = serde_json::from_str("\"draining\"").unwrap();
    assert_eq!(worker, WorkerStatus::Unknown("draining".to_owned()));

    let stop: StopReason = serde_json::from_str("\"content_filter\"").unwrap();
    assert_eq!(stop, StopReason::Unknown("content_filter".to_owned()));

    let trigger: TriggerKind = serde_json::from_str("\"nats_event\"").unwrap();
    assert_eq!(trigger, TriggerKind::Unknown("nats_event".to_owned()));

    let err: RpcErrorCode = serde_json::from_str("\"quota_exceeded\"").unwrap();
    assert_eq!(err, RpcErrorCode::Unknown("quota_exceeded".to_owned()));

    let env: EventEnvelope = serde_json::from_value(json!({
        "seq": 1,
        "emittedAtMs": 1000,
        "codec": "events.v1",
        "sessionId": "s1",
        "payload": { "kind": "someFutureEvent", "x": 1 }
    }))
    .unwrap();
    assert_eq!(env.payload, StreamPayload::Unknown);
}

#[test]
fn sandbox_matrix_enforces_fail_closed_combinations() {
    let valid_cases = [
        (
            SetupMode::Local,
            RuntimeKind::Docker,
            IsolationKind::Openshell,
            false,
        ),
        (
            SetupMode::Local,
            RuntimeKind::Docker,
            IsolationKind::Native,
            false,
        ),
        (
            SetupMode::Local,
            RuntimeKind::Docker,
            IsolationKind::None,
            false,
        ),
        (
            SetupMode::Local,
            RuntimeKind::Host,
            IsolationKind::None,
            false,
        ),
        (
            SetupMode::Local,
            RuntimeKind::Host,
            IsolationKind::Native,
            true,
        ),
        (
            SetupMode::Remote,
            RuntimeKind::RemoteVps,
            IsolationKind::Openshell,
            false,
        ),
        (
            SetupMode::Remote,
            RuntimeKind::ThirdParty(ThirdPartyProvider::Daytona),
            IsolationKind::None,
            false,
        ),
    ];

    for (mode, runtime, isolation, allow_host_native) in valid_cases {
        let profile = SandboxProfile {
            mode,
            runtime,
            isolation,
            allow_host_native,
            gateway_url: None,
        };
        assert!(profile.validate().is_ok());
    }

    let host_openshell = SandboxProfile {
        mode: SetupMode::Local,
        runtime: RuntimeKind::Host,
        isolation: IsolationKind::Openshell,
        allow_host_native: true,
        gateway_url: None,
    };
    assert_eq!(
        host_openshell.validate(),
        Err(SandboxProfileError::OpenshellRequiresContainerOrRemote)
    );

    let host_native_no_optin = SandboxProfile {
        mode: SetupMode::Local,
        runtime: RuntimeKind::Host,
        isolation: IsolationKind::Native,
        allow_host_native: false,
        gateway_url: None,
    };
    assert_eq!(
        host_native_no_optin.validate(),
        Err(SandboxProfileError::HostNativeRequiresExplicitOptIn)
    );

    let mode_mismatch = SandboxProfile {
        mode: SetupMode::Local,
        runtime: RuntimeKind::RemoteVps,
        isolation: IsolationKind::Openshell,
        allow_host_native: false,
        gateway_url: None,
    };
    assert_eq!(
        mode_mismatch.validate(),
        Err(SandboxProfileError::ModeRuntimeMismatch)
    );

    assert_eq!(
        SandboxProfile {
            mode: SetupMode::Local,
            runtime: RuntimeKind::Docker,
            isolation: IsolationKind::Openshell,
            allow_host_native: false,
            gateway_url: None,
        }
        .effective_label(),
        "local · docker + openshell"
    );
}

#[test]
fn swarm_permission_mask_blocks_privilege_escalation() {
    let parent = PermissionMask {
        can_read_fs: true,
        can_write_fs: true,
        can_exec_shell: false,
        can_network: false,
        allowed_tools: vec!["read_file".to_owned(), "write_file".to_owned()],
    };

    let valid_child = PermissionMask {
        can_read_fs: true,
        can_write_fs: false,
        can_exec_shell: false,
        can_network: false,
        allowed_tools: vec!["read_file".to_owned()],
    };
    assert!(parent.narrow(&valid_child).is_ok());

    let shell_escalation = PermissionMask {
        can_read_fs: true,
        can_write_fs: false,
        can_exec_shell: true,
        can_network: false,
        allowed_tools: vec!["read_file".to_owned()],
    };
    assert_eq!(
        parent.narrow(&shell_escalation),
        Err(PermissionError::PrivilegeEscalation {
            capability: "can_exec_shell",
        })
    );

    let tool_escalation = PermissionMask {
        can_read_fs: true,
        can_write_fs: true,
        can_exec_shell: false,
        can_network: false,
        allowed_tools: vec!["exec_bash".to_owned()],
    };
    assert_eq!(
        parent.narrow(&tool_escalation),
        Err(PermissionError::DisallowedTool {
            tool: "exec_bash".to_owned(),
        })
    );
}

#[test]
fn auth_structs_redact_secrets_in_debug_output() {
    let login: LoginRequest = serde_json::from_value(json!({
        "email": "user@example.com",
        "password": "my-ultra-secret-password"
    }))
    .unwrap();
    let refresh: RefreshRequest = serde_json::from_value(json!({
        "refreshToken": "refresh-secret-token"
    }))
    .unwrap();
    let pair = TokenPair {
        access_token: "jwt-secret-value".to_owned(),
        refresh_token: "refresh-secret-value".to_owned(),
        token_type: "Bearer".to_owned(),
        expires_in_seconds: 900,
    };

    assert!(!format!("{login:?}").contains("my-ultra-secret-password"));
    assert!(!format!("{refresh:?}").contains("refresh-secret-token"));
    assert!(!format!("{pair:?}").contains("jwt-secret-value"));
    assert!(!format!("{pair:?}").contains("refresh-secret-value"));
}

#[test]
fn artifact_manifest_validates_sha256_and_sizes() {
    let valid_hash = "a".repeat(64);
    let manifest = ArtifactManifest {
        artifact_id: "art_1".to_owned(),
        task_id: "tsk_1".to_owned(),
        files: vec![ArtifactEntry {
            path: "index.html".to_owned(),
            size_bytes: 1024,
            sha256: valid_hash.clone(),
            content_type: "text/html".to_owned(),
        }],
        total_bytes: 1024,
        created_at_ms: 100,
    };
    assert!(manifest.validate().is_ok());

    let bad_hash = ArtifactManifest {
        files: vec![ArtifactEntry {
            path: "index.html".to_owned(),
            size_bytes: 1024,
            sha256: "not-a-sha256".to_owned(),
            content_type: "text/html".to_owned(),
        }],
        ..manifest.clone()
    };
    assert_eq!(
        bad_hash.validate(),
        Err(ArtifactValidationError::InvalidSha256Hex {
            path: "index.html".to_owned()
        })
    );

    let bad_total = ArtifactManifest {
        total_bytes: 999,
        ..manifest
    };
    assert_eq!(
        bad_total.validate(),
        Err(ArtifactValidationError::TotalBytesMismatch {
            declared: 999,
            computed: 1024
        })
    );
}

#[derive(Deserialize)]
struct FilterQuery {
    #[serde(default, deserialize_with = "TriStateFilter::from_nullable_value")]
    tag: TriStateFilter<String>,
}

#[test]
fn tristate_filter_distinguishes_omitted_null_and_exact() {
    let omitted: FilterQuery = serde_json::from_str("{}").unwrap();
    assert_eq!(omitted.tag, TriStateFilter::Any);

    let explicit_null: FilterQuery = serde_json::from_str("{\"tag\": null}").unwrap();
    assert_eq!(explicit_null.tag, TriStateFilter::None);

    let exact: FilterQuery = serde_json::from_str("{\"tag\": \"prod\"}").unwrap();
    assert_eq!(exact.tag, TriStateFilter::Exact("prod".to_owned()));
}

#[test]
fn lean_rpc_request_omits_jsonrpc_and_schema_exports_catalog() {
    let req = RpcRequest {
        id: RequestId::Num(1),
        method: "task/submit".to_owned(),
        params: Some(json!({"taskId": "t1"})),
        trace: None,
    };
    let val = serde_json::to_value(req).unwrap();
    assert!(val.get("jsonrpc").is_none());

    let schema = export_protocol_schema();
    assert_eq!(schema.api_generation, API_GENERATION);
    assert_eq!(schema.rpc_methods.len(), METHOD_CATALOG.len());
    assert!(!schema.valid_sandbox_profiles.is_empty());
}
