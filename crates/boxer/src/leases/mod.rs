//! Fenced lease acquisition, periodic heartbeat supervisor, and RAII `DropGuard` cleanup.

pub mod client;
pub mod guard;
pub mod supervisor;

pub use client::{BoxFuture, HttpServerLeaseClient, InMemoryLeaseClient, LeaseClient};
pub use guard::{CleanupDropGuard, SandboxResources};
pub use supervisor::LeaseSupervisor;
