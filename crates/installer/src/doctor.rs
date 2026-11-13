use serde::{Deserialize, Serialize};

/// Outcome of one non-destructive diagnostic check reported by `90-doctor.sh`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DoctorStatus {
    Pass,
    Fail,
    Skip,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DoctorCheck {
    pub status: DoctorStatus,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct DoctorReport {
    pub checks: Vec<DoctorCheck>,
    pub passed: usize,
    pub failed: usize,
    pub skipped: usize,
}

impl DoctorReport {
    /// Parses `[PASS]`, `[FAIL]`, and `[SKIP]` lines from doctor stdout/stderr.
    #[must_use]
    pub fn parse(output: &str) -> Self {
        let mut report = Self::default();
        for line in output.lines() {
            let trimmed = line.trim();
            if let Some(msg) = trimmed.strip_prefix("[PASS]") {
                report.passed += 1;
                report.checks.push(DoctorCheck {
                    status: DoctorStatus::Pass,
                    message: redact_secrets(msg.trim()),
                });
            } else if let Some(msg) = trimmed.strip_prefix("[FAIL]") {
                report.failed += 1;
                report.checks.push(DoctorCheck {
                    status: DoctorStatus::Fail,
                    message: redact_secrets(msg.trim()),
                });
            } else if let Some(msg) = trimmed.strip_prefix("[SKIP]") {
                report.skipped += 1;
                report.checks.push(DoctorCheck {
                    status: DoctorStatus::Skip,
                    message: redact_secrets(msg.trim()),
                });
            }
        }
        report
    }

    #[must_use]
    pub const fn is_healthy(&self) -> bool {
        self.failed == 0
    }
}

/// Redacts sensitive environment key assignments, Postgres/Redis URLs, and NKey seeds from logs.
#[must_use]
pub fn redact_secrets(input: &str) -> String {
    const SECRET_KEYS: &[&str] = &[
        "AGENTS_JWT_SECRET=",
        "AGENTS_CREDENTIAL_MASTER_KEY=",
        "AGENTS_POSTGRES_PASSWORD=",
        "VALKEY_PASSWORD=",
        "NATS_NKEY_SEED=",
    ];

    input
        .lines()
        .map(|line| {
            let mut out = line.to_owned();
            for key in SECRET_KEYS {
                if let Some(idx) = out.find(key) {
                    let prefix_end = idx + key.len();
                    let rest = &out[prefix_end..];
                    let val_end = rest.find(char::is_whitespace).unwrap_or(rest.len());
                    out.replace_range(prefix_end..prefix_end + val_end, "[REDACTED]");
                }
            }
            for scheme in &["postgres://", "redis://"] {
                if let Some(idx) = out.find(scheme) {
                    let tail = &out[idx + scheme.len()..];
                    if let Some(at) = tail.find('@') {
                        let start = idx + scheme.len();
                        out.replace_range(start..start + at, "[REDACTED]");
                    }
                }
            }
            out
        })
        .collect::<Vec<_>>()
        .join("\n")
}
