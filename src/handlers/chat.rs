use std::convert::Infallible;
use std::sync::Arc;
use std::time::Duration;

use axum::extract::State;
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use axum::Json;
use futures::StreamExt;

use crate::copilot::types::ChatCompletionRequest;
use crate::error::AppError;
use crate::server::AppState;

/// Handler for `POST /v1/chat/completions`.
///
/// For non-streaming requests (`stream: false` or absent), forwards the request
/// to the Copilot API and returns the complete JSON response.
/// For streaming requests (`stream: true`), returns Server-Sent Events.
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

    // Get a valid Copilot token
    let copilot_token = state
        .token_manager
        .get_valid_token()
        .await
        .map_err(|e| AppError::Unauthorized(e.to_string()))?;

    // Branch on stream field
    if request.stream.unwrap_or(false) {
        let chunk_stream = state
            .copilot_client
            .stream_chat_completion(&copilot_token, &request)
            .await?;

        // Map each ChatCompletionChunk into an SSE Event
        let event_stream = chunk_stream.map(|result| -> Result<Event, Infallible> {
            match result {
                Ok(chunk) => {
                    let json = serde_json::to_string(&chunk).unwrap_or_default();
                    Ok(Event::default().data(json))
                }
                Err(e) => {
                    let err_json = serde_json::json!({
                        "error": { "message": e.to_string(), "type": "stream_error" }
                    });
                    Ok(Event::default().data(err_json.to_string()))
                }
            }
        });

        // Append a final [DONE] event after all chunks
        let done_stream = futures::stream::once(async {
            Ok::<Event, Infallible>(Event::default().data("[DONE]"))
        });
        let full_stream = event_stream.chain(done_stream);

        let sse = Sse::new(full_stream).keep_alive(
            KeepAlive::new().interval(Duration::from_secs(15)),
        );

        return Ok(sse.into_response());
    }

    // Non-streaming: forward to the Copilot API
    let response = state
        .copilot_client
        .send_chat_completion(&copilot_token, &request)
        .await?;

    Ok(Json(response).into_response())
}
