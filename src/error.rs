use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde_json::json;

/// Application error types following the OpenAI-compatible error format.
///
/// Each variant maps to an HTTP status code and produces a JSON response:
/// ```json
/// { "error": { "message": "...", "type": "...", "code": "..." } }
/// ```
#[derive(thiserror::Error, Debug)]
pub enum AppError {
    #[error("Authentication required")]
    NotAuthenticated,

    #[error("Token expired")]
    TokenExpired,

    #[error("GitHub API error: {0}")]
    GitHubError(String),

    #[error("Copilot API error: {0}")]
    CopilotError(String),

    #[error("Rate limited, retry after {0}s")]
    RateLimited(u64),

    #[error("Invalid request: {0}")]
    InvalidRequest(String),

    #[error("Not found: {0}")]
    ModelNotFound(String),

    #[error("prompt is too long: {actual_tokens} tokens > {limit_tokens} maximum")]
    PromptTooLong {
        actual_tokens: u32,
        limit_tokens: u32,
    },

    #[error("Internal error: {0}")]
    Internal(String),
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let (status, error_response) = match &self {
            AppError::NotAuthenticated => (
                StatusCode::UNAUTHORIZED,
                json!({
                    "error": {
                        "message": self.to_string(),
                        "type": "authentication_error",
                        "code": "not_authenticated"
                    }
                }),
            ),
            AppError::TokenExpired => (
                StatusCode::UNAUTHORIZED,
                json!({
                    "error": {
                        "message": self.to_string(),
                        "type": "authentication_error",
                        "code": "token_expired"
                    }
                }),
            ),
            AppError::GitHubError(msg) => (
                StatusCode::BAD_GATEWAY,
                json!({
                    "error": {
                        "message": msg,
                        "type": "upstream_error",
                        "code": "github_error"
                    }
                }),
            ),
            AppError::CopilotError(msg) => (
                StatusCode::BAD_GATEWAY,
                json!({
                    "error": {
                        "message": msg,
                        "type": "upstream_error",
                        "code": "copilot_error"
                    }
                }),
            ),
            AppError::RateLimited(secs) => (
                StatusCode::TOO_MANY_REQUESTS,
                json!({
                    "error": {
                        "message": self.to_string(),
                        "type": "rate_limit_error",
                        "code": "rate_limited",
                        "retry_after": secs
                    }
                }),
            ),
            AppError::InvalidRequest(msg) => (
                StatusCode::BAD_REQUEST,
                json!({
                    "error": {
                        "message": msg,
                        "type": "invalid_request_error",
                        "code": "invalid_request"
                    }
                }),
            ),
            AppError::ModelNotFound(msg) => (
                StatusCode::NOT_FOUND,
                json!({
                    "error": {
                        "message": msg,
                        "type": "not_found_error",
                        "code": "model_not_found"
                    }
                }),
            ),
            AppError::PromptTooLong { .. } => (
                StatusCode::BAD_REQUEST,
                json!({
                    "error": {
                        "message": self.to_string(),
                        "type": "invalid_request_error",
                        "code": "prompt_too_long"
                    }
                }),
            ),
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
        };

        tracing::warn!(
            error_type = %self.error_type(),
            status = %status.as_u16(),
            "{}",
            self
        );

        // For rate-limited errors, include the Retry-After HTTP header.
        if let AppError::RateLimited(secs) = &self {
            let mut resp = (status, Json(error_response)).into_response();
            resp.headers_mut().insert(
                "Retry-After",
                axum::http::HeaderValue::from_str(&secs.to_string())
                    .unwrap_or_else(|_| axum::http::HeaderValue::from_static("60")),
            );
            return resp;
        }

        (status, Json(error_response)).into_response()
    }
}

impl AppError {
    /// Returns the OpenAI error type string for this error.
    pub fn error_type(&self) -> &'static str {
        match self {
            AppError::NotAuthenticated | AppError::TokenExpired => "authentication_error",
            AppError::GitHubError(_) | AppError::CopilotError(_) => "upstream_error",
            AppError::RateLimited(_) => "rate_limit_error",
            AppError::InvalidRequest(_) | AppError::PromptTooLong { .. } => "invalid_request_error",
            AppError::ModelNotFound(_) => "not_found_error",
            AppError::Internal(_) => "internal_error",
        }
    }
}

impl From<anyhow::Error> for AppError {
    fn from(err: anyhow::Error) -> Self {
        AppError::Internal(err.to_string())
    }
}
