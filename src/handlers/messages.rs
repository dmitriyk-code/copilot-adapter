use std::convert::Infallible;
use std::sync::Arc;
use std::time::Duration;

use axum::extract::State;
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use axum::Json;
use futures::StreamExt;

use crate::anthropic::types::{
    build_message_start_response, map_stop_reason, AnthropicRequest, MessageDeltaBody,
    MessageDeltaUsage, ResponseContentBlock, StreamEvent, TextDelta,
};
use crate::error::AppError;
use crate::server::AppState;

/// Handler for `POST /v1/messages`.
///
/// Accepts Anthropic Messages API requests, translates them to OpenAI format,
/// forwards to the Copilot API, and translates the response back to Anthropic
/// format. Supports both streaming and non-streaming modes.
pub async fn messages(
    State(state): State<Arc<AppState>>,
    Json(request): Json<AnthropicRequest>,
) -> Result<Response, AppError> {
    // Validate: messages must be non-empty
    if request.messages.is_empty() {
        return Err(AppError::InvalidRequest(
            "messages must be a non-empty array".to_string(),
        ));
    }

    // Translate Anthropic request to OpenAI format
    let openai_request = request.to_chat_completion_request();

    // Get a valid Copilot token
    let copilot_token = state
        .token_manager
        .get_valid_token()
        .await
        .map_err(|e| {
            tracing::warn!(error = %e, "Authentication failed");
            AppError::NotAuthenticated
        })?;

    // Branch on stream field
    if request.stream.unwrap_or(false) {
        return handle_streaming(state, &copilot_token, &openai_request).await;
    }

    // Non-streaming: forward to the Copilot API and translate response
    let response = state
        .copilot_client
        .send_chat_completion(&copilot_token, &openai_request)
        .await?;

    let anthropic_response = response.to_anthropic_response();
    Ok(Json(anthropic_response).into_response())
}

/// Handle a streaming Anthropic Messages API request.
///
/// Translates OpenAI SSE chunks into Anthropic-format streaming events:
/// `message_start` → `content_block_start` → `content_block_delta`* →
/// `content_block_stop` → `message_delta` → `message_stop`.
async fn handle_streaming(
    state: Arc<AppState>,
    copilot_token: &str,
    openai_request: &crate::copilot::types::ChatCompletionRequest,
) -> Result<Response, AppError> {
    let chunk_stream = state
        .copilot_client
        .stream_chat_completion(copilot_token, openai_request)
        .await?;

    let model = openai_request.model.clone();

    let event_stream = async_stream::stream! {
        let mut content_block_opened = false;
        let mut message_started = false;
        let mut last_finish_reason: Option<String> = None;

        let mut stream = std::pin::pin!(chunk_stream);

        while let Some(result) = stream.next().await {
            match result {
                Ok(chunk) => {
                    if !message_started {
                        let msg = build_message_start_response(&chunk.id, &model);
                        let event = StreamEvent::MessageStart { message: msg };
                        let json = match serde_json::to_string(&event) {
                            Ok(j) => j,
                            Err(e) => {
                                tracing::error!("failed to serialise SSE event: {e}");
                                return;
                            }
                        };
                        yield Ok::<Event, Infallible>(
                            Event::default().event("message_start").data(json)
                        );
                        message_started = true;
                    }

                    let mut serialisation_failed = false;
                    for choice in &chunk.choices {
                        // Handle text content deltas
                        if let Some(ref text) = choice.delta.content {
                            if !content_block_opened {
                                let event = StreamEvent::ContentBlockStart {
                                    index: 0,
                                    content_block: ResponseContentBlock {
                                        block_type: "text".to_string(),
                                        text: String::new(),
                                    },
                                };
                                let json = match serde_json::to_string(&event) {
                                    Ok(j) => j,
                                    Err(e) => {
                                        tracing::error!("failed to serialise SSE event: {e}");
                                        serialisation_failed = true;
                                        break;
                                    }
                                };
                                yield Ok(Event::default().event("content_block_start").data(json));
                                content_block_opened = true;
                            }

                            let event = StreamEvent::ContentBlockDelta {
                                index: 0,
                                delta: TextDelta {
                                    delta_type: "text_delta".to_string(),
                                    text: text.clone(),
                                },
                            };
                            let json = match serde_json::to_string(&event) {
                                Ok(j) => j,
                                Err(e) => {
                                    tracing::error!("failed to serialise SSE event: {e}");
                                    serialisation_failed = true;
                                    break;
                                }
                            };
                            yield Ok(Event::default().event("content_block_delta").data(json));
                        }

                        // Track finish_reason for the final message_delta
                        if choice.finish_reason.is_some() {
                            last_finish_reason = choice.finish_reason.clone();
                        }
                    }
                    if serialisation_failed {
                        return;
                    }
                }
                Err(e) => {
                    tracing::error!(error = %e, "Error in upstream stream");
                    break;
                }
            }
        }

        // Close the content block if one was opened
        if content_block_opened {
            let event = StreamEvent::ContentBlockStop { index: 0 };
            let json = match serde_json::to_string(&event) {
                Ok(j) => j,
                Err(e) => {
                    tracing::error!("failed to serialise SSE event: {e}");
                    return;
                }
            };
            yield Ok(Event::default().event("content_block_stop").data(json));
        }

        // Emit message_delta with stop reason and usage.
        // output_tokens is set to 0 because the upstream OpenAI SSE stream does not
        // include per-chunk token counts, so an accurate count is unavailable during
        // streaming. Reporting 0 is less misleading than an incorrect positive number.
        if message_started {
            let stop_reason = map_stop_reason(last_finish_reason.as_deref());
            let event = StreamEvent::MessageDelta {
                delta: MessageDeltaBody {
                    stop_reason,
                    stop_sequence: None,
                },
                usage: MessageDeltaUsage { output_tokens: 0 },
            };
            let json = match serde_json::to_string(&event) {
                Ok(j) => j,
                Err(e) => {
                    tracing::error!("failed to serialise SSE event: {e}");
                    return;
                }
            };
            yield Ok(Event::default().event("message_delta").data(json));

            // Emit message_stop
            let event = StreamEvent::MessageStop {};
            let json = match serde_json::to_string(&event) {
                Ok(j) => j,
                Err(e) => {
                    tracing::error!("failed to serialise SSE event: {e}");
                    return;
                }
            };
            yield Ok(Event::default().event("message_stop").data(json));
        }
    };

    let sse = Sse::new(event_stream).keep_alive(
        KeepAlive::new().interval(Duration::from_secs(15)),
    );

    Ok(sse.into_response())
}
