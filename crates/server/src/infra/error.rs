//! Unified server domain errors mapped cleanly to Ironic `HttpError` and RPC codes.

use agents_protocol::RpcErrorCode;
use ironic::{HttpError, HttpStatus};
use thiserror::Error;

/// Domain-level server errors translated to safe HTTP or RPC responses.
#[derive(Debug, Error)]
pub enum ServerError {
    #[error("unauthorized: {0}")]
    Unauthorized(String),
    #[error("forbidden: {0}")]
    Forbidden(String),
    #[error("not found: {0}")]
    NotFound(String),
    #[error("conflict: {0}")]
    Conflict(String),
    #[error("validation failed: {0}")]
    Validation(String),
    #[error("rate limit exceeded: {0}")]
    RateLimited(String),
    #[error("server overloaded: {0}")]
    Overloaded(String),
    #[error("internal error: {0}")]
    Internal(String),
}

impl ServerError {
    /// Maps this server error into a safe client-facing [`HttpError`].
    #[must_use]
    pub fn into_http(self) -> HttpError {
        match self {
            Self::Unauthorized(msg) => HttpError::unauthorized("AUTH_UNAUTHORIZED", msg),
            Self::Forbidden(msg) => HttpError::forbidden("AUTH_FORBIDDEN", msg),
            Self::NotFound(msg) => HttpError::not_found("RESOURCE_NOT_FOUND", msg),
            Self::Conflict(msg) => HttpError::new(HttpStatus::CONFLICT, "STATE_CONFLICT", msg),
            Self::Validation(msg) => HttpError::unprocessable_entity("VALIDATION_FAILED", msg),
            Self::RateLimited(msg) => {
                HttpError::new(HttpStatus::TOO_MANY_REQUESTS, "RATE_LIMITED", msg)
            }
            Self::Overloaded(msg) => {
                HttpError::new(HttpStatus::SERVICE_UNAVAILABLE, "SERVER_OVERLOADED", msg)
            }
            Self::Internal(_) => {
                HttpError::internal("INTERNAL_ERROR", "An internal error occurred")
            }
        }
    }

    /// Maps this server error into a typed protocol [`RpcErrorCode`].
    #[must_use]
    pub const fn rpc_code(&self) -> RpcErrorCode {
        match self {
            Self::Unauthorized(_) => RpcErrorCode::Unauthorized,
            Self::Forbidden(_) => RpcErrorCode::Forbidden,
            Self::NotFound(_) => RpcErrorCode::NotFound,
            Self::Conflict(_) => RpcErrorCode::Conflict,
            Self::Validation(_) => RpcErrorCode::InvalidParams,
            Self::RateLimited(_) | Self::Overloaded(_) => RpcErrorCode::Overloaded,
            Self::Internal(_) => RpcErrorCode::Internal,
        }
    }
}

impl From<ServerError> for HttpError {
    fn from(value: ServerError) -> Self {
        value.into_http()
    }
}

/// Validates a `garde` DTO and maps validation errors to [`HttpError`] 422.
///
/// # Errors
///
/// Returns `422 Unprocessable Entity` with formatted validation violations.
pub fn validate_dto<T>(dto: &T) -> Result<(), HttpError>
where
    T: garde::Validate,
    <T as garde::Validate>::Context: Default,
{
    dto.validate()
        .map_err(|report| HttpError::unprocessable_entity("VALIDATION_FAILED", report.to_string()))
}

/// Standard structured error response payload rendered across all OpenAPI 4xx/5xx operations.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ErrorResponse {
    /// Machine-readable error code string.
    pub error: String,
    /// Detailed human-readable description or field-level validation report.
    pub message: String,
}

impl ironic::OpenApiSchema for ErrorResponse {
    fn openapi_schema() -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "title": "ErrorResponse",
            "description": "Standard structured error envelope",
            "properties": {
                "error": {
                    "type": "string",
                    "description": "Machine-readable error code string",
                    "example": "VALIDATION_FAILED"
                },
                "message": {
                    "type": "string",
                    "description": "Human-readable explanation of the error or validation breakdown",
                    "example": "Validation failed: field 'email' is invalid"
                }
            },
            "required": ["error", "message"]
        })
    }
}
