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
        };

        (status, Json(error_response)).into_response()
    }
}

impl From<anyhow::Error> for AppError {
    fn from(err: anyhow::Error) -> Self {
        AppError::Internal(err.to_string())
    }
}
