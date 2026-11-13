use thiserror::Error;

/// Typed errors returned by the installer planner, validator, and executor.
#[derive(Debug, Error)]
pub enum InstallerError {
    #[error("invalid topology or configuration: {0}")]
    Validation(String),

    #[error("missing deployment script: {0}")]
    MissingScript(String),

    #[error("execution failed ({script}, exit={code:?}): {message}")]
    Execution {
        script: String,
        code: Option<i32>,
        message: String,
    },

    #[error("i/o failure: {0}")]
    Io(#[from] std::io::Error),
}
