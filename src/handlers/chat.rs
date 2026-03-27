use std::convert::Infallible;
use std::sync::Arc;
use std::time::Duration;

use axum::extract::State;
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use axum::Json;
use futures::StreamExt;

use crate::copilot::types::{ChatCompletionRequest, MessageContent};
use crate::error::AppError;
use crate::server::AppState;
use crate::tools::injector;
use crate::tools::parser;

/// Handler for `POST /v1/chat/completions`.
///
/// For non-streaming requests (`stream: false` or absent), forwards the request
/// to the Copilot API and returns the complete JSON response.
/// For streaming requests (`stream: true`), returns Server-Sent Events.
///
/// When `--experimental-tools` is enabled and the request contains tool
/// definitions, they are injected into the system prompt and tool calls are
/// parsed from the model's text response.
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

    let has_tools = request
        .tools
        .as_ref()
        .map_or(false, |t| !t.is_empty());

    let has_tool_role = request.messages.iter().any(|m| m.role == "tool");

    // If the request uses tools but the feature is disabled, reject early.
    if (has_tools || has_tool_role) && !state.config.experimental_tools {
        return Err(AppError::InvalidRequest(
            "Tool/function calling is not supported. Start the adapter with \
             --experimental-tools to enable experimental tool support via \
             prompt injection."
                .to_string(),
        ));
    }

    // Build the request to send upstream, applying tool injection if needed.
    let mut upstream_request = request.clone();

    if state.config.experimental_tools {
        // Inject tool definitions into the system prompt.
        if let Some(ref tools) = request.tools {
            if !tools.is_empty() {
                injector::inject_tools_into_messages(&mut upstream_request.messages, tools);
            }
        }

        // Translate tool-role messages into user messages.
        injector::translate_tool_messages(&mut upstream_request.messages);
    }

    // Strip tools/tool_choice — Copilot API does not accept them.
    upstream_request.tools = None;
    upstream_request.tool_choice = None;

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
    if upstream_request.stream.unwrap_or(false) {
        let chunk_stream = state
            .copilot_client
            .stream_chat_completion(&copilot_token, &upstream_request)
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
    let mut response = state
        .copilot_client
        .send_chat_completion(&copilot_token, &upstream_request)
        .await?;

    // Post-process: parse tool calls from the response content when tools were
    // requested and the experimental flag is enabled.
    if has_tools && state.config.experimental_tools {
        for choice in &mut response.choices {
            let content_text = choice.message.content.as_text();
            let tool_calls = parser::parse_tool_calls(&content_text);

            if !tool_calls.is_empty() {
                let stripped = parser::strip_tool_calls(&content_text);
                choice.message.content = if stripped.is_empty() {
                    MessageContent::Text(String::new())
                } else {
                    MessageContent::Text(stripped)
                };
                choice.message.tool_calls = Some(tool_calls);
                // When tool calls are present, signal the client that the
                // model wants to invoke tools rather than stopping normally.
                choice.finish_reason = Some("tool_calls".to_string());
            }
        }
    }

    Ok(Json(response).into_response())
}
