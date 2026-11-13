#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::{collections::BTreeMap, sync::Arc};

use agents_client::core::{
    BackendTarget, InMemorySecretStore, MockTransport, OtaPackageAsset, OtaSlotStore,
    PlatformAsset, ReleaseKeyCertificate, ReleaseManifest, RevocationList, ServerModeProvider,
    SignedOtaPackage, TrialDecision, UpdateError, UpdateKind, UpdaterConfig, check_manifest,
    key_id_from_b64, sha256_hex,
    updater::{certificate_signing_message, package_signing_message},
    verify_download_bytes, verify_signed_nshpkg,
};
use agents_client::ui::ClientController;
use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
use ed25519_dalek::{Signer, SigningKey};

fn temp_dir(label: &str) -> std::path::PathBuf {
    let p = std::env::temp_dir().join(format!(
        "agents-updater-{label}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&p).unwrap();
    p
}

#[test]
fn signed_manifest_verifies_ed25519_blocks_downgrade_and_skips_nightly_prerelease() {
    let update_sk = SigningKey::from_bytes(&[7u8; 32]);
    let update_pk_b64 = BASE64.encode(update_sk.verifying_key().to_bytes());

    let archive = b"binary-archive-payload-v0.2.0";
    let archive_sha = sha256_hex(archive);
    let mut platforms = BTreeMap::new();
    platforms.insert(
        "macos-aarch64".to_owned(),
        PlatformAsset {
            url:
                "https://github.com/natuleadan/agents/releases/download/v0.2.0/agents-client.tar.gz"
                    .to_owned(),
            sha256: archive_sha.clone(),
            size: archive.len() as u64,
        },
    );
    platforms.insert(
        "android-apk".to_owned(),
        PlatformAsset {
            url: "https://play.google.com/store/apps/details?id=dev.natuleadan.agents".to_owned(),
            sha256: archive_sha.clone(),
            size: archive.len() as u64,
        },
    );

    let prod_manifest = ReleaseManifest {
        id: "dev.natuleadan.agents".to_owned(),
        version: "0.2.0".to_owned(),
        notes: "Signed release v0.2.0".to_owned(),
        prerelease: false,
        platforms,
        package: None,
    };
    let prod_bytes = serde_json::to_vec(&prod_manifest).unwrap();
    let prod_sig = BASE64.encode(update_sk.sign(&prod_bytes).to_bytes());

    let cfg = UpdaterConfig {
        app_id: "dev.natuleadan.agents".to_owned(),
        current_version: "0.1.0".to_owned(),
        current_build: 1,
        current_runtime: "0.1.0".to_owned(),
        platform_key: "macos-aarch64".to_owned(),
        update_public_key_b64: Some(update_pk_b64.clone()),
        trusted_root_pubkeys_b64: Vec::new(),
        highest_accepted_build: 1,
        can_self_install: true,
        allow_prerelease: false,
    };

    let available = check_manifest(&cfg, &prod_bytes, Some(&prod_sig))
        .unwrap()
        .unwrap();
    assert_eq!(available.kind, UpdateKind::AppBinary);
    assert!(available.can_install);
    assert_eq!(available.version, "0.2.0");
    assert!(verify_download_bytes(archive, &archive_sha, archive.len() as u64).is_ok());
    assert!(matches!(
        verify_download_bytes(b"tampered", &archive_sha, archive.len() as u64),
        Err(UpdateError::SizeMismatch { .. })
    ));

    // Tampered manifest fails Ed25519 verification
    let mut tampered = prod_bytes.clone();
    tampered[0] = b' ';
    assert_eq!(
        check_manifest(&cfg, &tampered, Some(&prod_sig)),
        Err(UpdateError::InvalidSignature)
    );

    // Prerelease (dev nightly) is ignored by production updater
    let mut nightly = prod_manifest.clone();
    nightly.prerelease = true;
    let nightly_bytes = serde_json::to_vec(&nightly).unwrap();
    let nightly_sig = BASE64.encode(update_sk.sign(&nightly_bytes).to_bytes());
    assert_eq!(
        check_manifest(&cfg, &nightly_bytes, Some(&nightly_sig)),
        Ok(None)
    );

    // Downgrade is ignored cleanly
    let mut older_cfg = cfg.clone();
    older_cfg.current_version = "0.3.0".to_owned();
    assert_eq!(
        check_manifest(&older_cfg, &prod_bytes, Some(&prod_sig)),
        Ok(None)
    );

    // Mobile store-link fallback sets can_install = false
    let mut android_cfg = cfg.clone();
    android_cfg.platform_key = "android-apk".to_owned();
    android_cfg.can_self_install = false;
    let mob = check_manifest(&android_cfg, &prod_bytes, Some(&prod_sig))
        .unwrap()
        .unwrap();
    assert_eq!(mob.kind, UpdateKind::StoreLink);
    assert!(!mob.can_install);

    // Controller wires manifest update into AppState UI
    let transport = Arc::new(MockTransport::default());
    let provider = Arc::new(ServerModeProvider::new(
        "https://agents.example.com",
        transport,
    ));
    let mut controller = ClientController::new(
        provider,
        Arc::new(InMemorySecretStore::default()),
        BackendTarget::local_server("https://agents.example.com"),
    );
    assert!(
        controller
            .evaluate_release_manifest(&cfg, &prod_bytes, Some(&prod_sig))
            .unwrap()
            .is_some()
    );
    assert_eq!(
        controller.state.update_info.available_version.as_deref(),
        Some("0.2.0")
    );
}

#[test]
fn signed_nshpkg_two_level_chain_revocation_and_crash_rollback_work() {
    let root_sk = SigningKey::from_bytes(&[11u8; 32]);
    let root_pk_b64 = BASE64.encode(root_sk.verifying_key().to_bytes());
    let rel_sk = SigningKey::from_bytes(&[22u8; 32]);
    let rel_pk_b64 = BASE64.encode(rel_sk.verifying_key().to_bytes());

    let mut cert = ReleaseKeyCertificate {
        root_pubkey_b64: root_pk_b64.clone(),
        release_pubkey_b64: rel_pk_b64.clone(),
        app_pattern: "dev.natuleadan.*".to_owned(),
        expires_at_epoch: 2_000_000_000,
        root_sig_b64: String::new(),
    };
    cert.root_sig_b64 = BASE64.encode(root_sk.sign(&certificate_signing_message(&cert)).to_bytes());

    let make_pkg = |build: u64, ver: &str, payload: &[u8]| -> Vec<u8> {
        let mut pkg = SignedOtaPackage {
            app_id: "dev.natuleadan.agents".to_owned(),
            version: ver.to_owned(),
            build,
            min_runtime: "0.1.0".to_owned(),
            signed_at_epoch: 1_750_000_000,
            payload_sha256: sha256_hex(payload),
            payload_b64: BASE64.encode(payload),
            certificate: cert.clone(),
            release_sig_b64: String::new(),
        };
        pkg.release_sig_b64 = BASE64.encode(rel_sk.sign(&package_signing_message(&pkg)).to_bytes());
        serde_json::to_vec(&pkg).unwrap()
    };

    let cfg = UpdaterConfig {
        app_id: "dev.natuleadan.agents".to_owned(),
        current_version: "0.1.0".to_owned(),
        current_build: 1,
        current_runtime: "0.2.0".to_owned(),
        platform_key: "macos-aarch64".to_owned(),
        update_public_key_b64: None,
        trusted_root_pubkeys_b64: vec![root_pk_b64],
        highest_accepted_build: 1,
        can_self_install: true,
        allow_prerelease: false,
    };

    let pkg2 = make_pkg(2, "0.1.1", b"ui-bundle-build-2");
    let pkg3 = make_pkg(3, "0.1.2", b"bad-ui-bundle-build-3");
    let revs = RevocationList::default();
    let dir = temp_dir("ota-slots");
    let store = OtaSlotStore::new(&dir);

    // Stage build 2 -> trial -> confirm healthy -> current.nshpkg
    store.stage_next(&cfg, &pkg2, &revs, 1_750_000_100).unwrap();
    assert_eq!(
        store
            .begin_startup_trial(&cfg, &revs, 1_750_000_100)
            .unwrap(),
        TrialDecision::RunTrying {
            build: 2,
            version: "0.1.1".to_owned()
        }
    );
    store.confirm_trial_healthy(2).unwrap();

    // Rollback protection rejects build 2 now that build 2 is already the accepted floor
    assert_eq!(
        verify_signed_nshpkg(
            &UpdaterConfig {
                highest_accepted_build: 2,
                ..cfg.clone()
            },
            &pkg2,
            &revs,
            1_750_000_200,
        ),
        Err(UpdateError::RollbackRejected {
            candidate: 2,
            floor: 2
        })
    );

    // Stage build 3 -> start trial -> simulate crash (next boot does NOT call confirm_trial_healthy)
    store.stage_next(&cfg, &pkg3, &revs, 1_750_000_200).unwrap();
    assert_eq!(
        store
            .begin_startup_trial(&cfg, &revs, 1_750_000_200)
            .unwrap(),
        TrialDecision::RunTrying {
            build: 3,
            version: "0.1.2".to_owned()
        }
    );
    // Crash occurs before confirm_trial_healthy! Next boot deletes trying.nshpkg and rolls back to current (build 2)!
    assert_eq!(
        store
            .begin_startup_trial(&cfg, &revs, 1_750_000_300)
            .unwrap(),
        TrialDecision::RunCurrent {
            build: 2,
            version: "0.1.1".to_owned()
        }
    );

    // Revocation rejects previously valid key
    let rel_key_id = key_id_from_b64(&rel_pk_b64).unwrap();
    let rev_list = RevocationList {
        sequence: 1,
        revoked_key_ids: vec![rel_key_id.clone()],
        revoked_package_sha256: Vec::new(),
    };
    assert_eq!(
        verify_signed_nshpkg(&cfg, &pkg3, &rev_list, 1_750_000_300),
        Err(UpdateError::Revoked(format!("release key {rel_key_id}")))
    );

    // Loopback HTTP allowed; plain remote HTTP rejected in manifest check
    let bad_manifest = serde_json::to_vec(&ReleaseManifest {
        id: "dev.natuleadan.agents".to_owned(),
        version: "0.9.0".to_owned(),
        notes: "bad".to_owned(),
        prerelease: false,
        platforms: BTreeMap::new(),
        package: Some(OtaPackageAsset {
            version: "0.9.0".to_owned(),
            build: 99,
            url: "http://evil.example.com/pkg.nshpkg".to_owned(),
            sha256: "00".repeat(32),
            size: 10,
            min_runtime: "0.1.0".to_owned(),
        }),
    })
    .unwrap();
    assert!(matches!(
        check_manifest(&cfg, &bad_manifest, None),
        Err(UpdateError::InsecureUrl(_))
    ));

    let _ = std::fs::remove_dir_all(&dir);
}
