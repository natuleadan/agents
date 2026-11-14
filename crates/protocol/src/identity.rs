//! Shared installation-level agent identity headers and validation helpers.

use serde::{Deserialize, Serialize};

/// HTTP header carrying the organization name that owns the agent installation.
pub const HEADER_AGENT_ORGANIZATION: &str = "x-agent-organization";

/// HTTP header carrying the unique alphanumeric installation identifier.
pub const HEADER_AGENT_INSTALLATION_ID: &str = "x-agent-installation-id";

/// HTTP header carrying the optional signed identity certificate/code bound to the installation IP.
pub const HEADER_AGENT_IDENTITY_CODE: &str = "x-agent-identity-code";

/// Validates that an installation identifier is non-empty, 4..=64 ASCII alphanumeric/dash/underscore chars.
#[must_use]
pub fn is_valid_alphanumeric_id(value: &str) -> bool {
    let trimmed = value.trim();
    (4..=64).contains(&trimmed.len())
        && trimmed
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

/// Validates that an organization identifier is non-empty and within 2..=128 printable ASCII chars.
#[must_use]
pub fn is_valid_organization_name(value: &str) -> bool {
    let trimmed = value.trim();
    (2..=128).contains(&trimmed.len()) && trimmed.bytes().all(|b| b.is_ascii_graphic() || b == b' ')
}

/// Installation-wide identity headers injected on outbound traffic and inspected on inbound traffic.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InstallationIdentityHeaders {
    pub organization: String,
    pub installation_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub identity_code: Option<String>,
}

impl InstallationIdentityHeaders {
    /// Returns `true` when both organization name and alphanumeric installation ID are well-formed.
    #[must_use]
    pub fn is_complete(&self) -> bool {
        is_valid_organization_name(&self.organization)
            && is_valid_alphanumeric_id(&self.installation_id)
    }
}
