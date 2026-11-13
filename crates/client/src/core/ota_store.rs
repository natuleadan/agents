use std::{
    fs,
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};

use super::updater::{
    RevocationList, SignedOtaPackage, UpdateError, UpdaterConfig, verify_signed_nshpkg,
};

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct TrustState {
    pub highest_accepted_build: u64,
    pub revocation_sequence: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TrialDecision {
    RunTrying { build: u64, version: String },
    RunCurrent { build: u64, version: String },
    RunBundled,
}

/// Manages `next.nshpkg` -> `trying.nshpkg` -> `current.nshpkg` OTA slots with crash rollback.
pub struct OtaSlotStore {
    root_dir: PathBuf,
}

impl OtaSlotStore {
    #[must_use]
    pub fn new(root_dir: impl AsRef<Path>) -> Self {
        Self {
            root_dir: root_dir.as_ref().to_path_buf(),
        }
    }

    fn next_path(&self) -> PathBuf {
        self.root_dir.join("next.nshpkg")
    }

    fn trying_path(&self) -> PathBuf {
        self.root_dir.join("trying.nshpkg")
    }

    fn current_path(&self) -> PathBuf {
        self.root_dir.join("current.nshpkg")
    }

    fn trust_path(&self) -> PathBuf {
        self.root_dir.join("trust/state.json")
    }

    pub fn load_trust(&self) -> Result<TrustState, UpdateError> {
        let p = self.trust_path();
        if !p.exists() {
            return Ok(TrustState::default());
        }
        let raw = fs::read(p).map_err(|e| UpdateError::Io(e.to_string()))?;
        serde_json::from_slice(&raw).map_err(|e| UpdateError::Json(e.to_string()))
    }

    fn save_trust(&self, state: &TrustState) -> Result<(), UpdateError> {
        let p = self.trust_path();
        if let Some(parent) = p.parent() {
            fs::create_dir_all(parent).map_err(|e| UpdateError::Io(e.to_string()))?;
        }
        let bytes =
            serde_json::to_vec_pretty(state).map_err(|e| UpdateError::Json(e.to_string()))?;
        fs::write(p, bytes).map_err(|e| UpdateError::Io(e.to_string()))
    }

    /// Verifies and stages a downloaded `.nshpkg` as `next.nshpkg`.
    pub fn stage_next(
        &self,
        cfg: &UpdaterConfig,
        pkg_bytes: &[u8],
        revocations: &RevocationList,
        now_epoch: u64,
    ) -> Result<SignedOtaPackage, UpdateError> {
        let trust = self.load_trust()?;
        let mut effective_cfg = cfg.clone();
        effective_cfg.highest_accepted_build = effective_cfg
            .highest_accepted_build
            .max(trust.highest_accepted_build);
        let parsed = verify_signed_nshpkg(&effective_cfg, pkg_bytes, revocations, now_epoch)?;
        fs::create_dir_all(&self.root_dir).map_err(|e| UpdateError::Io(e.to_string()))?;
        fs::write(self.next_path(), pkg_bytes).map_err(|e| UpdateError::Io(e.to_string()))?;
        Ok(parsed)
    }

    /// Startup sequence matching NSH `ota-updates.md`: deletes crashed `trying.nshpkg`, promotes `next.nshpkg`, or falls back to `current.nshpkg` / bundled.
    pub fn begin_startup_trial(
        &self,
        cfg: &UpdaterConfig,
        revocations: &RevocationList,
        now_epoch: u64,
    ) -> Result<TrialDecision, UpdateError> {
        if self.trying_path().exists() {
            let _ = fs::remove_file(self.trying_path());
        }
        let trust = self.load_trust()?;
        let mut effective_cfg = cfg.clone();
        effective_cfg.highest_accepted_build = effective_cfg
            .highest_accepted_build
            .max(trust.highest_accepted_build);

        if self.next_path().exists() {
            let bytes = fs::read(self.next_path()).map_err(|e| UpdateError::Io(e.to_string()))?;
            if let Ok(pkg) = verify_signed_nshpkg(&effective_cfg, &bytes, revocations, now_epoch) {
                fs::rename(self.next_path(), self.trying_path())
                    .map_err(|e| UpdateError::Io(e.to_string()))?;
                return Ok(TrialDecision::RunTrying {
                    build: pkg.build,
                    version: pkg.version,
                });
            }
            let _ = fs::remove_file(self.next_path());
        }

        if self.current_path().exists() {
            let bytes =
                fs::read(self.current_path()).map_err(|e| UpdateError::Io(e.to_string()))?;
            let mut floor_cfg = effective_cfg;
            floor_cfg.highest_accepted_build = floor_cfg.highest_accepted_build.saturating_sub(1);
            if let Ok(pkg) = verify_signed_nshpkg(&floor_cfg, &bytes, revocations, now_epoch)
                && pkg.build > cfg.current_build
            {
                return Ok(TrialDecision::RunCurrent {
                    build: pkg.build,
                    version: pkg.version,
                });
            }
            let _ = fs::remove_file(self.current_path());
        }

        Ok(TrialDecision::RunBundled)
    }

    /// Promotes `trying.nshpkg` to `current.nshpkg` and raises the anti-rollback build floor.
    pub fn confirm_trial_healthy(&self, build: u64) -> Result<(), UpdateError> {
        if self.trying_path().exists() {
            fs::rename(self.trying_path(), self.current_path())
                .map_err(|e| UpdateError::Io(e.to_string()))?;
        }
        let mut trust = self.load_trust()?;
        trust.highest_accepted_build = trust.highest_accepted_build.max(build);
        self.save_trust(&trust)
    }
}
