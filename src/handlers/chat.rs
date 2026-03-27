use std::sync::Arc;

use axum::extract::State;
use axum::response::{IntoResponse, Response};
use axum::Json;

use crate::copilot::types::ChatCompletionRequest;
use crate::error::AppError;
use crate::server::AppState;

/// Handler for `POST /v1/chat/completions`.
///
/// For non-streaming requests (`stream: false` or absent), forwards the request
/// to the Copilot API and returns the complete JSON response.
/// Streaming (`stream: true`) is not yet implemented (Epic 4).
pub async fn chat_completions(
    State(state): State<Arc<AppState>>,
    Json(request): Json<ChatCompletionRequest>,
) -> Result<Response, AppError> {
    // Validate: messages must be non-empty
    if request.messages.is_empty() {
        return Err(AppError::InvalidRequest(
            "messages must be a non-empty array".to_string(),
        ));
    }

    // Branch on stream field
    if request.stream.unwrap_or(false) {
        // Streaming will be implemented in Epic 4
        return Err(AppError::InvalidRequest(
            "Streaming is not yet supported. Set \"stream\": false or omit the field."
                .to_string(),
        ));
    }

    // Get a valid Copilot token
    let copilot_token = state
        .token_manager
        .get_valid_token()
        .await
        .map_err(|e| AppError::Unauthorized(e.to_string()))?;

    // Forward to the Copilot API
    let response = state
        .copilot_client
        .send_chat_completion(&copilot_token, &request)
        .await?;

    Ok(Json(response).into_response())
}
