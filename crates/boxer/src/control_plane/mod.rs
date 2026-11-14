//! Boxer HTTP control plane: provisioning, VFS access, metrics, and autodiscovery.

pub mod discovery;
pub mod dto;
pub mod openapi;
pub mod server;
pub mod service;

pub use discovery::start_autodiscovery_heartbeat;
pub use dto::{
    BoxStatusResponse, BoxerHealthResponse, BoxerProblemDetails, ExecBoxRequest, ExecBoxResponse,
    ProvisionBoxRequest, ProvisionBoxResponse, ReadFileResponse, TeardownBoxResponse,
    WriteFileRequest,
};
pub use server::build_control_plane_router;
pub use service::BoxerControlPlaneService;
