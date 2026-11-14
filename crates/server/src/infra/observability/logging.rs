//! Structured logging, TimeSeriesLayer JSONL rotation, OTLP initialization, and log querying.

use std::{
    fs::OpenOptions,
    io::{BufRead, BufReader},
    path::{Path, PathBuf},
    sync::Arc,
};

use ironic::{
    OpenApiSchema,
    logging::{FileLogStorage, TimeSeriesConfig},
    telemetry::{TelemetryConfig, TracingGuard, init_tracing},
};
use serde::{Deserialize, Serialize};
use tracing_subscriber::{EnvFilter, Layer, layer::SubscriberExt, util::SubscriberInitExt};

use super::apm::ApmConfig;
use crate::infra::adapters::BlobStorePort;

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

    /// Ensures the log destination directory exists and is writable, falling back to `.logs` if needed.
    #[must_use]
    pub fn resolve_writable_dir(&self) -> PathBuf {
        if ensure_writable(&self.log_dir) {
            return self.log_dir.clone();
        }
        let fallback = PathBuf::from(".logs");
        let _ = ensure_writable(&fallback);
        fallback
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

fn ensure_writable(dir: &Path) -> bool {
    if !dir.exists() && std::fs::create_dir_all(dir).is_err() {
        return false;
    }
    let probe = dir.join(".write_probe");
    if OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(true)
        .open(&probe)
        .is_ok()
    {
        let _ = std::fs::remove_file(&probe);
        true
    } else {
        false
    }
}

/// Initializes structured logging (`fmt` + Ironic `TimeSeriesLayer` -> `YYYY-MM-DD.jsonl`)
/// and optional OTLP distributed tracing when `AGENTS_OTEL_ENDPOINT` / `OTLP_ENDPOINT` is set.
#[must_use]
pub fn init_structured_logging(logging: &LoggingConfig, apm: &ApmConfig) -> Option<TracingGuard> {
    if let Some(endpoint) = &apm.otlp_endpoint {
        let guard = init_tracing(TelemetryConfig {
            service_name: apm.service_name.clone(),
            otlp_endpoint: Some(endpoint.clone()),
            batch_interval: apm.batch_interval(),
            sample_rate: apm.sample_rate,
            propagate_context: apm.propagate_context,
        });
        return Some(guard);
    }

    let writable_dir = logging.resolve_writable_dir();
    let storage = Arc::new(FileLogStorage::new(&writable_dir));
    let ts_layer = TimeSeriesConfig::default().with_storage(storage).layer();
    let ts_filter =
        EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(&logging.level));
    let fmt_filter =
        EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(&logging.level));

    let _ = tracing_subscriber::registry()
        .with(ts_layer.with_filter(ts_filter))
        .with(tracing_subscriber::fmt::layer().with_filter(fmt_filter))
        .try_init();

    None
}

/// Query parameters for filtering structured JSONL logs.
#[derive(Clone, Debug, Default, Deserialize, Serialize, garde::Validate)]
pub struct SystemLogQueryDto {
    /// Filter by log level (`ERROR`, `WARN`, `INFO`, `DEBUG`, `TRACE`).
    #[garde(length(max = 16))]
    pub level: Option<String>,
    /// Substring match on the tracing `target` module path.
    #[garde(length(max = 128))]
    pub target: Option<String>,
    /// Free-text substring match across message, trace_id, or fields.
    #[garde(length(max = 128))]
    pub q: Option<String>,
    /// Maximum number of log entries to return (default 100, capped at 500).
    #[garde(range(min = 1, max = 500))]
    pub limit: Option<usize>,
}

/// Response payload returned by the structured system log query endpoint.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SystemLogsResponse {
    /// Matching log entries ordered newest-first.
    pub logs: Vec<serde_json::Value>,
    /// Number of returned entries.
    pub total: usize,
    /// Active log directory scanned.
    pub log_dir: String,
}

impl OpenApiSchema for SystemLogsResponse {
    fn openapi_schema() -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "required": ["logs", "total", "log_dir"],
            "properties": {
                "logs": { "type": "array", "items": { "type": "object" } },
                "total": { "type": "integer", "example": 25 },
                "log_dir": { "type": "string", "example": "/var/log/agents" }
            }
        })
    }
}

/// Queries daily `.jsonl` structured logs from `log_dir`, returning newest matching entries first.
#[must_use]
pub fn query_jsonl_logs(log_dir: &Path, query: &SystemLogQueryDto) -> SystemLogsResponse {
    let limit = query.limit.unwrap_or(100).clamp(1, 500);
    let level_filter = query.level.as_deref().map(str::to_ascii_uppercase);
    let target_filter = query.target.as_deref().map(str::to_ascii_lowercase);
    let q_filter = query.q.as_deref().map(str::to_ascii_lowercase);

    let mut files: Vec<PathBuf> = std::fs::read_dir(log_dir)
        .into_iter()
        .flatten()
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| p.extension().and_then(|s| s.to_str()) == Some("jsonl"))
        .collect();
    files.sort();
    files.reverse();

    let mut matches = Vec::new();
    for path in files {
        let Ok(file) = std::fs::File::open(&path) else {
            continue;
        };
        let lines: Vec<String> = BufReader::new(file).lines().map_while(Result::ok).collect();
        for line in lines.into_iter().rev() {
            if line.trim().is_empty() {
                continue;
            }
            let Ok(val) = serde_json::from_str::<serde_json::Value>(&line) else {
                continue;
            };
            if let Some(ref lvl) = level_filter {
                let entry_lvl = val["level"].as_str().unwrap_or("").to_ascii_uppercase();
                if &entry_lvl != lvl {
                    continue;
                }
            }
            if let Some(ref tgt) = target_filter {
                let entry_tgt = val["target"].as_str().unwrap_or("").to_ascii_lowercase();
                if !entry_tgt.contains(tgt) {
                    continue;
                }
            }
            if let Some(ref q) = q_filter
                && !line.to_ascii_lowercase().contains(q)
            {
                continue;
            }
            matches.push(val);
            if matches.len() >= limit {
                break;
            }
        }
        if matches.len() >= limit {
            break;
        }
    }

    let total = matches.len();
    SystemLogsResponse {
        logs: matches,
        total,
        log_dir: log_dir.display().to_string(),
    }
}

/// Archives `.jsonl` log files from `log_dir` into [`BlobStorePort`] (S3 / Upstash / LocalFS).
pub async fn archive_logs_to_blob(log_dir: &Path, blob: &dyn BlobStorePort) -> usize {
    let mut archived = 0;
    let Ok(entries) = std::fs::read_dir(log_dir) else {
        return 0;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|s| s.to_str()) != Some("jsonl") {
            continue;
        }
        let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        let Ok(bytes) = std::fs::read(&path) else {
            continue;
        };
        let key = format!("observability/logs/{name}");
        if blob
            .put_blob(&key, bytes, "application/x-ndjson")
            .await
            .is_ok()
        {
            archived += 1;
        }
    }
    archived
}
