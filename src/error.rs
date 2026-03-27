use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde_json::json;

#[derive(thiserror::Error, Debug)]
pub enum AppError {
    #[error("Internal error: {0}")]
    Internal(String),

    #[error("Invalid request: {0}")]
    InvalidRequest(String),

    #[error("Unauthorized: {0}")]
    Unauthorized(String),

    #[error("Not found: {0}")]
    NotFound(String),

    #[error("Upstream error: {0}")]
    UpstreamError(String),
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let (status, error_response) = match &self {
            AppError::Internal(msg) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                json!({
                    "error": {
                        "message": msg,
                        "type": "internal_error",
                        "code": "internal_error"
                    }
                }),
            ),
            AppError::InvalidRequest(msg) => (
                StatusCode::BAD_REQUEST,
                json!({
                    "error": {
                        "message": msg,
                        "type": "invalid_request_error",
                        "code": "invalid_request_error"
                    }
                }),
            ),
            AppError::Unauthorized(msg) => (
                StatusCode::UNAUTHORIZED,
                json!({
                    "error": {
                        "message": msg,
                        "type": "authentication_error",
                        "code": "unauthorized"
                    }
                }),
            ),
            AppError::NotFound(msg) => (
                StatusCode::NOT_FOUND,
                json!({
                    "error": {
                        "message": msg,
                        "type": "not_found_error",
                        "code": "model_not_found"
                    }
                }),
            ),
            AppError::UpstreamError(msg) => (
                StatusCode::BAD_GATEWAY,
                json!({
                    "error": {
                        "message": msg,
                        "type": "upstream_error",
                        "code": "upstream_error"
                    }
                }),
            ),
        };

        (status, Json(error_response)).into_response()
    }
}

impl From<anyhow::Error> for AppError {
    fn from(err: anyhow::Error) -> Self {
        AppError::Internal(err.to_string())
    }
}
