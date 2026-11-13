//! SHA-256 verified artifact manifest contracts delivered from Boxer to Server.

use std::fmt;

use serde::{Deserialize, Serialize};

/// Validation error for malformed artifact manifests or SHA-256 digests.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ArtifactValidationError {
    InvalidSha256Hex { path: String },
    TotalBytesMismatch { declared: u64, computed: u64 },
    EmptyPath,
}

impl fmt::Display for ArtifactValidationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidSha256Hex { path } => {
                write!(f, "invalid sha256 digest for artifact path '{path}'")
            }
            Self::TotalBytesMismatch { declared, computed } => {
                write!(
                    f,
                    "declared total_bytes ({declared}) != computed sum ({computed})"
                )
            }
            Self::EmptyPath => write!(f, "artifact entry path must not be empty"),
        }
    }
}

impl std::error::Error for ArtifactValidationError {}

/// Single file entry inside an artifact delivery bundle.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ArtifactEntry {
    pub path: String,
    pub size_bytes: u64,
    pub sha256: String,
    pub content_type: String,
}

/// Complete verified manifest for deliverables produced by a sandbox task.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ArtifactManifest {
    pub artifact_id: String,
    pub task_id: String,
    pub files: Vec<ArtifactEntry>,
    pub total_bytes: u64,
    pub created_at_ms: u64,
}

impl ArtifactManifest {
    /// Verifies digest hex formatting, non-empty paths, and byte sum consistency.
    pub fn validate(&self) -> Result<(), ArtifactValidationError> {
        let mut computed: u64 = 0;
        for entry in &self.files {
            if entry.path.trim().is_empty() {
                return Err(ArtifactValidationError::EmptyPath);
            }
            if !is_valid_sha256_hex(&entry.sha256) {
                return Err(ArtifactValidationError::InvalidSha256Hex {
                    path: entry.path.clone(),
                });
            }
            computed = computed.saturating_add(entry.size_bytes);
        }
        if computed != self.total_bytes {
            return Err(ArtifactValidationError::TotalBytesMismatch {
                declared: self.total_bytes,
                computed,
            });
        }
        Ok(())
    }
}

/// Checks whether a string is a 64-character lowercase/uppercase hexadecimal SHA-256 digest.
pub fn is_valid_sha256_hex(digest: &str) -> bool {
    digest.len() == 64 && digest.bytes().all(|b| b.is_ascii_hexdigit())
}
