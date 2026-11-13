//! SHA-256 verified artifact packager enforcing size quotas and symlink escape protection.

use std::path::{Path, PathBuf};

use agents_protocol::{ArtifactEntry, ArtifactManifest};
use sha2::{Digest, Sha256};

use crate::{config::now_ms, error::BoxerError};

/// Bounded quota configuration for packaging sandbox deliverables.
#[derive(Debug, Clone, Copy)]
pub struct ArtifactQuota {
    pub max_files: usize,
    pub max_file_bytes: u64,
    pub max_total_bytes: u64,
}

impl Default for ArtifactQuota {
    fn default() -> Self {
        Self {
            max_files: 512,
            max_file_bytes: 32 * 1024 * 1024,
            max_total_bytes: 128 * 1024 * 1024,
        }
    }
}

/// Packages files inside a confined sandbox output directory into an [`ArtifactManifest`].
#[derive(Debug, Clone, Copy, Default)]
pub struct ArtifactPackager {
    quota: ArtifactQuota,
}

impl ArtifactPackager {
    #[must_use]
    pub fn new(quota: ArtifactQuota) -> Self {
        Self { quota }
    }

    /// Walks `output_dir` recursively without following symlinks, hashes each file with SHA-256,
    /// enforces byte quotas, and returns a validated [`ArtifactManifest`].
    ///
    /// # Errors
    ///
    /// Returns [`BoxerError::Artifact`] on symlink encounters or quota breaches,
    /// or [`BoxerError::ArtifactValidation`] if integrity checks fail.
    pub fn package_directory(
        &self,
        task_id: &str,
        output_dir: &Path,
    ) -> Result<ArtifactManifest, BoxerError> {
        let canonical_root = std::fs::canonicalize(output_dir)?;
        if !canonical_root.is_dir() {
            return Err(BoxerError::Artifact(format!(
                "`{}` is not a directory",
                canonical_root.display()
            )));
        }

        let mut files: Vec<PathBuf> = Vec::new();
        collect_files_no_symlinks(&canonical_root, &canonical_root, &mut files)?;
        files.sort();

        if files.len() > self.quota.max_files {
            return Err(BoxerError::Artifact(format!(
                "artifact file count {} exceeds quota {}",
                files.len(),
                self.quota.max_files
            )));
        }

        let mut entries = Vec::with_capacity(files.len());
        let mut total_bytes: u64 = 0;

        for abs_path in files {
            let bytes = std::fs::read(&abs_path)?;
            let size_bytes = u64::try_from(bytes.len())
                .map_err(|_| BoxerError::Artifact("file size overflow".into()))?;
            if size_bytes > self.quota.max_file_bytes {
                return Err(BoxerError::Artifact(format!(
                    "artifact `{}` ({size_bytes} bytes) exceeds per-file quota {}",
                    abs_path.display(),
                    self.quota.max_file_bytes
                )));
            }
            total_bytes = total_bytes.saturating_add(size_bytes);
            if total_bytes > self.quota.max_total_bytes {
                return Err(BoxerError::Artifact(format!(
                    "total artifact size ({total_bytes} bytes) exceeds quota {}",
                    self.quota.max_total_bytes
                )));
            }

            let rel = abs_path
                .strip_prefix(&canonical_root)
                .map_err(|_| BoxerError::Artifact("path prefix strip failed".into()))?;
            let rel_str = rel
                .components()
                .map(|c| c.as_os_str().to_string_lossy().into_owned())
                .collect::<Vec<_>>()
                .join("/");

            let sha256 = sha256_hex(&bytes);
            let content_type = detect_content_type(&abs_path).to_owned();

            entries.push(ArtifactEntry {
                path: rel_str,
                size_bytes,
                sha256,
                content_type,
            });
        }

        let manifest = ArtifactManifest {
            artifact_id: uuid::Uuid::new_v4().to_string(),
            task_id: task_id.to_owned(),
            files: entries,
            total_bytes,
            created_at_ms: now_ms(),
        };
        manifest.validate()?;
        Ok(manifest)
    }
}

fn collect_files_no_symlinks(
    root: &Path,
    current: &Path,
    out: &mut Vec<PathBuf>,
) -> Result<(), BoxerError> {
    for entry in std::fs::read_dir(current)? {
        let entry = entry?;
        let path = entry.path();
        let meta = std::fs::symlink_metadata(&path)?;
        if meta.file_type().is_symlink() {
            return Err(BoxerError::Artifact(format!(
                "refusing symlink `{}` during artifact packaging",
                path.display()
            )));
        }
        if meta.is_dir() {
            collect_files_no_symlinks(root, &path, out)?;
        } else if meta.is_file() {
            let canonical = std::fs::canonicalize(&path)?;
            if !canonical.starts_with(root) {
                return Err(BoxerError::Artifact(format!(
                    "artifact file `{}` escapes output directory",
                    canonical.display()
                )));
            }
            out.push(canonical);
        }
    }
    Ok(())
}

/// Computes lowercase hex-encoded SHA-256 digest of arbitrary bytes.
#[must_use]
pub fn sha256_hex(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    hex::encode(hasher.finalize())
}

fn detect_content_type(path: &Path) -> &'static str {
    let ext = path
        .extension()
        .and_then(|s| s.to_str())
        .map(str::to_ascii_lowercase);
    match ext.as_deref() {
        Some("html" | "htm") => "text/html",
        Some("css") => "text/css",
        Some("js" | "mjs") => "application/javascript",
        Some("json") => "application/json",
        Some("md") => "text/markdown",
        Some("txt" | "log") => "text/plain",
        Some("svg") => "image/svg+xml",
        Some("png") => "image/png",
        Some("jpg" | "jpeg") => "image/jpeg",
        _ => "application/octet-stream",
    }
}
