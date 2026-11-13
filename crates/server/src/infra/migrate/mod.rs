//! Data migration engine between Topología A (Self-Hosted VPS) and Topología B (Cloud-Native VPS).

pub mod export;
pub mod import;
pub mod types;
pub mod verify;

pub use export::{export_snapshot, validate_in_memory_audit_chain};
pub use import::import_snapshot;
pub use types::{
    ImportSummary, MigrateApiKeyRecord, MigrateAutocronRecord, MigrateRenderedSlugRecord,
    MigrateSessionRecord, MigrateUserRecord, MigrateWorkflowRecord, MigrationSnapshot,
    VerifyReport,
};
pub use verify::verify_database;
