//! Validation DTOs for querying audit logs and triggering retention archival.

use garde::Validate;
use ironic::OpenApiSchema;
use serde::{Deserialize, Serialize};

/// Query parameters for filtering recent audit entries.
#[derive(Debug, Clone, Default, Deserialize, Validate)]
#[serde(rename_all = "camelCase")]
pub struct AuditQueryDto {
    #[garde(length(max = 128))]
    pub actor_id: Option<String>,
    #[garde(length(max = 128))]
    pub action: Option<String>,
    #[garde(range(min = 1, max = 200))]
    pub limit: Option<u32>,
}

/// Summary of audit chain verification.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, OpenApiSchema)]
#[serde(rename_all = "camelCase")]
pub struct AuditVerificationReport {
    pub checked_entries: usize,
    pub valid: bool,
    pub broken_entry_id: Option<String>,
}
