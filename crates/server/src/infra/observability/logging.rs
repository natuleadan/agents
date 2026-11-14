//! Structured logging and JSON Lines rotation correlated with W3C trace identifiers.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// Configuration for structured file-backed time-series logging.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LoggingConfig {
    /// Directory where daily `.jsonl` log files are stored.
    pub log_dir: PathBuf,
    /// Minimum log level filter (`trace`, `debug`, `info`, `warn`, `error`).
    pub level: String,
    /// Whether structured events should also emit to stdout.
    pub stdout_enabled: bool,
}

impl Default for LoggingConfig {
    fn default() -> Self {
        let log_dir = std::env::var("AGENTS_LOG_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|_| {
                if Path::new("/var/log/agents").is_dir() {
                    PathBuf::from("/var/log/agents")
                } else {
                    PathBuf::from(".logs")
                }
            });

        let level =
            std::env::var("RUST_LOG").unwrap_or_else(|_| "info,agents_server=debug".to_owned());

        Self {
            log_dir,
            level,
            stdout_enabled: true,
        }
    }
}

impl LoggingConfig {
    /// Creates a custom logging configuration pointing to `dir`.
    #[must_use]
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        Self {
            log_dir: dir.into(),
            level: "info".to_owned(),
            stdout_enabled: true,
        }
    }

    /// Resolves the expected path for today's log file (`YYYY-MM-DD.jsonl`).
    #[must_use]
    pub fn today_log_file(&self) -> PathBuf {
        let now = chrono::Utc::now();
        let filename = format!("{}.jsonl", now.format("%Y-%m-%d"));
        self.log_dir.join(filename)
    }

    /// Ensures the log destination directory exists.
    ///
    /// # Errors
    ///
    /// Returns [`std::io::Error`] if creating the target directory fails.
    pub fn ensure_dir(&self) -> Result<(), std::io::Error> {
        if !self.log_dir.exists() {
            std::fs::create_dir_all(&self.log_dir)?;
        }
        Ok(())
    }
}
