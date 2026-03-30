use std::convert::Infallible;
use std::sync::Arc;
use std::time::Duration;

use axum::extract::State;
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use axum::Json;
use futures::StreamExt;

use crate::copilot::types::{
    ChatCompletionChunk, ChatCompletionRequest, ChunkChoice, ChunkDelta, MessageContent,
};
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
/// When the request contains tool definitions, they are injected into the system
/// prompt and tool calls are parsed from the model's text response.
pub async fn chat_completions(
    State(state): State<Arc<AppState>>,
    Json(request): Json<ChatCompletionRequest>,
) -> Result<Response, AppError> {
    tracing::debug!(
        model = %request.model,
        stream = ?request.stream,
        num_messages = request.messages.len(),
        "Received chat completion request"
    );

    // TRACE: Log the full incoming request from Claude Code
    if tracing::enabled!(tracing::Level::TRACE) {
        if let Ok(json) = serde_json::to_string_pretty(&request) {
            tracing::trace!(
                direction = "INCOMING",
                source = "Claude Code",
                endpoint = "/v1/chat/completions",
                request_json = %json,
                "Full request received from Claude Code"
            );
        }
    }

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

    // Check if any messages have the "tool" role (indicating tool results).
    let has_tool_role = request.messages.iter().any(|m| m.role == "tool");

    // Build the request to send upstream, applying tool injection if needed.
    let mut upstream_request = request.clone();

    // Normalize the model name to match GitHub Copilot's expected format
    upstream_request.model = crate::model_mapper::normalize_model_name(&upstream_request.model);

    if upstream_request.model != request.model {
        tracing::info!(
            original_model = %request.model,
            normalized_model = %upstream_request.model,
            "Model name normalized for GitHub Copilot compatibility"
        );
    }

    // Inject tool definitions into the system prompt.
    // IMPORTANT: Only inject if tools are explicitly provided in this request.
    // Claude Code is responsible for re-sending tool definitions on every turn
    // when tool calling is active (including turns with tool-role messages).
    if let Some(ref tools) = request.tools {
        if !tools.is_empty() {
            tracing::debug!(
                num_tools = tools.len(),
                tool_names = ?tools.iter().map(|t| &t.function.name).collect::<Vec<_>>(),
                "Injecting tools into prompt"
            );
            injector::inject_tools_into_messages(&mut upstream_request.messages, tools);
        }
    } else if has_tool_role {
        // Tool-role messages are present but no tool definitions were provided.
        // This is likely a bug in the client (Claude Code should re-send tool
        // definitions on every turn). Log a warning to help debug.
        tracing::warn!(
            "Request contains tool-role messages but no tool definitions. \
             The model may generate malformed tool calls without schema context. \
             Claude Code should re-send tool definitions on every turn when tool \
             calling is active."
        );
    }

    // Translate tool-role messages into user messages.
    injector::translate_tool_messages(&mut upstream_request.messages);

    // Strip tools/tool_choice — Copilot API does not accept them.
    upstream_request.tools = None;
    upstream_request.tool_choice = None;

    // TRACE: Log the full request being sent to GitHub Copilot API
    if tracing::enabled!(tracing::Level::TRACE) {
        if let Ok(json) = serde_json::to_string_pretty(&upstream_request) {
            tracing::trace!(
                direction = "OUTGOING",
                destination = "GitHub Copilot API",
                endpoint = "/chat/completions",
                request_json = %json,
                "Full request being sent to GitHub Copilot API"
            );
        }
    }

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
        // TRACE: Log that we're initiating a streaming request
        if tracing::enabled!(tracing::Level::TRACE) {
            tracing::trace!(
                direction = "OUTGOING",
                destination = "GitHub Copilot API",
                endpoint = "/chat/completions",
                mode = "streaming",
                "Initiating streaming request to GitHub Copilot API"
            );
        }

        let chunk_stream = state
            .copilot_client
            .stream_chat_completion(&copilot_token, &upstream_request)
            .await?;

        let parse_tools = has_tools;

        if parse_tools {
            // Buffer all chunks, detect tool calls at stream end, and emit
            // structured tool_calls in the final chunk.
            return handle_streaming_with_tools(chunk_stream).await;
        }

        // Normal streaming path (no tools) — pass through chunks unmodified.
        let event_stream = chunk_stream.map(|result| -> Result<Event, Infallible> {
            match result {
                Ok(chunk) => {
                    // TRACE: Log each chunk received from GitHub Copilot
                    if tracing::enabled!(tracing::Level::TRACE) {
                        if let Ok(json) = serde_json::to_string(&chunk) {
                            tracing::trace!(
                                direction = "INCOMING",
                                source = "GitHub Copilot API",
                                mode = "streaming",
                                chunk_json = %json,
                                "Received SSE chunk from GitHub Copilot API"
                            );
                        }
                    }

                    match serde_json::to_string(&chunk) {
                        Ok(json) => Ok(Event::default().data(json)),
                        Err(e) => {
                            tracing::warn!("Failed to serialize SSE chunk: {e}");
                            let err_json = serde_json::json!({
                                "error": { "message": format!("Serialization error: {e}"), "type": "stream_error" }
                            });
                            Ok(Event::default().data(err_json.to_string()))
                        }
                    }
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

    // TRACE: Log the full response received from GitHub Copilot API
    if tracing::enabled!(tracing::Level::TRACE) {
        if let Ok(json) = serde_json::to_string_pretty(&response) {
            tracing::trace!(
                direction = "INCOMING",
                source = "GitHub Copilot API",
                endpoint = "/chat/completions",
                response_json = %json,
                "Full response received from GitHub Copilot API"
            );
        }
    }

    // Log the raw response for debugging tool call issues
    tracing::debug!(
        actual_model = %response.model,
        "Actual model used by Copilot (from non-streaming response)"
    );

    // TRACE level: dump full response JSON to see exact structure
    if tracing::enabled!(tracing::Level::TRACE) {
        if let Ok(json) = serde_json::to_string_pretty(&response) {
            tracing::trace!(response_json = %json, "Full response JSON from Copilot");
        }
    }

    for (idx, choice) in response.choices.iter().enumerate() {
        let content_text = choice.message.content.as_text();
        tracing::debug!(
            choice_index = idx,
            content_length = content_text.len(),
            content_preview = %content_text.chars().take(200).collect::<String>(),
            finish_reason = ?choice.finish_reason,
            existing_tool_calls = ?choice.message.tool_calls,
            "Raw response from Copilot (chat endpoint)"
        );

        // If content is not too long, log it fully at trace level
        if tracing::enabled!(tracing::Level::TRACE) && content_text.len() < 2000 {
            tracing::trace!(
                choice_index = idx,
                full_content = %content_text,
                "Full content text from Copilot response"
            );
        }
    }

    // Post-process: parse tool calls from the response content when tools were requested.
    if has_tools {
        for choice in &mut response.choices {
            let content_text = choice.message.content.as_text();
            let tool_calls = parser::parse_tool_calls(&content_text);

            if !tool_calls.is_empty() {
                tracing::debug!(
                    num_tool_calls = tool_calls.len(),
                    tool_call_names = ?tool_calls.iter().map(|tc| &tc.function.name).collect::<Vec<_>>(),
                    "Parsed tool calls from response"
                );
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

    // TRACE: Log the final response being sent back to Claude Code
    if tracing::enabled!(tracing::Level::TRACE) {
        if let Ok(json) = serde_json::to_string_pretty(&response) {
            tracing::trace!(
                direction = "OUTGOING",
                destination = "Claude Code",
                endpoint = "/v1/chat/completions",
                response_json = %json,
                "Final response being sent to Claude Code"
            );
        }
    }

    Ok(Json(response).into_response())
}

/// Handle streaming responses when tool definitions are present.
///
/// Buffers all upstream SSE chunks, accumulates text content, and at stream
/// end parses tool calls from the accumulated text. If tool calls are found,
/// emits synthetic chunks with stripped text content and structured `tool_calls`
/// in the delta. If no tool calls are detected, replays the buffered chunks
/// as-is so the client sees the original streaming behaviour.
async fn handle_streaming_with_tools(
    chunk_stream: impl futures::Stream<Item = Result<ChatCompletionChunk, AppError>>
        + Send
        + 'static,
) -> Result<Response, AppError> {
    let event_stream = async_stream::stream! {
        let mut buffered_chunks: Vec<ChatCompletionChunk> = Vec::new();
        let mut content_buffer = String::new();
        let mut stream = std::pin::pin!(chunk_stream);

        while let Some(result) = stream.next().await {
            match result {
                Ok(chunk) => {
                    // TRACE: Log each chunk received from GitHub Copilot
                    if tracing::enabled!(tracing::Level::TRACE) {
                        if let Ok(json) = serde_json::to_string(&chunk) {
                            tracing::trace!(
                                direction = "INCOMING",
                                source = "GitHub Copilot API",
                                mode = "streaming_with_tools",
                                chunk_json = %json,
                                "Received SSE chunk from GitHub Copilot API (buffering for tool parsing)"
                            );
                        }
                    }

                    for choice in &chunk.choices {
                        if let Some(ref text) = choice.delta.content {
                            content_buffer.push_str(text);
                        }
                    }
                    buffered_chunks.push(chunk);
                }
                Err(e) => {
                    let err_json = serde_json::json!({
                        "error": { "message": e.to_string(), "type": "stream_error" }
                    });
                    yield Ok::<Event, Infallible>(Event::default().data(err_json.to_string()));
                    return;
                }
            }
        }

        // Stream ended — check for tool calls in the accumulated text
        tracing::debug!(
            content_length = content_buffer.len(),
            "Streaming response complete (OpenAI format), checking for tool calls"
        );

        // Get actual model from first chunk
        let actual_model = buffered_chunks
            .first()
            .map(|c| c.model.as_str())
            .unwrap_or("unknown");

        tracing::debug!(
            actual_model = actual_model,
            "Actual model used by Copilot (from response)"
        );

        // Log raw content for debugging
        if tracing::enabled!(tracing::Level::TRACE) {
            if content_buffer.len() < 2000 {
                tracing::trace!(
                    full_content = %content_buffer,
                    "Full buffered content from streaming response (OpenAI)"
                );
            } else {
                tracing::trace!(
                    content_preview = %content_buffer.chars().take(500).collect::<String>(),
                    content_length = content_buffer.len(),
                    "Buffered content preview (truncated, OpenAI)"
                );
            }
        } else {
            tracing::debug!(
                content_preview = %content_buffer.chars().take(200).collect::<String>(),
                "Buffered content preview (OpenAI)"
            );
        }

        let tool_calls = parser::parse_tool_calls(&content_buffer);

        if tool_calls.is_empty() {
            tracing::debug!("No tool calls found in streaming response (OpenAI)");
            // No tool calls detected — replay buffered chunks unchanged
            for chunk in &buffered_chunks {
                match serde_json::to_string(chunk) {
                    Ok(json) => yield Ok(Event::default().data(json)),
                    Err(e) => {
                        tracing::warn!("Failed to serialize buffered SSE chunk: {e}");
                        continue;
                    }
                }
            }
        } else {
            // Tool calls found — emit stripped content + tool_calls chunk
            tracing::debug!(
                num_tool_calls = tool_calls.len(),
                tool_call_names = ?tool_calls.iter().map(|tc| &tc.function.name).collect::<Vec<_>>(),
                "Parsed tool calls from streaming response (OpenAI)"
            );

            let stripped = parser::strip_tool_calls(&content_buffer);

            // Use metadata from the first buffered chunk
            if let Some(first) = buffered_chunks.first() {
                // Emit initial role chunk
                let role_chunk = ChatCompletionChunk {
                    id: first.id.clone(),
                    object: first.object.clone(),
                    created: first.created,
                    model: first.model.clone(),
                    choices: vec![ChunkChoice {
                        index: 0,
                        delta: ChunkDelta {
                            role: Some("assistant".to_string()),
                            content: None,
                            tool_calls: None,
                        },
                        finish_reason: None,
                    }],
                };

                // TRACE: Log synthetic chunk being sent
                if tracing::enabled!(tracing::Level::TRACE) {
                    if let Ok(json) = serde_json::to_string(&role_chunk) {
                        tracing::trace!(
                            direction = "OUTGOING",
                            destination = "Claude Code",
                            mode = "streaming_with_tools",
                            chunk_type = "role",
                            chunk_json = %json,
                            "Sending synthetic role chunk to Claude Code"
                        );
                    }
                }

                match serde_json::to_string(&role_chunk) {
                    Ok(json) => yield Ok(Event::default().data(json)),
                    Err(e) => {
                        tracing::warn!("Failed to serialize role chunk: {e}");
                    }
                }

                // Emit stripped text content (if any remains after stripping)
                if !stripped.is_empty() {
                    let text_chunk = ChatCompletionChunk {
                        id: first.id.clone(),
                        object: first.object.clone(),
                        created: first.created,
                        model: first.model.clone(),
                        choices: vec![ChunkChoice {
                            index: 0,
                            delta: ChunkDelta {
                                role: None,
                                content: Some(stripped),
                                tool_calls: None,
                            },
                            finish_reason: None,
                        }],
                    };

                    // TRACE: Log synthetic text chunk being sent
                    if tracing::enabled!(tracing::Level::TRACE) {
                        if let Ok(json) = serde_json::to_string(&text_chunk) {
                            tracing::trace!(
                                direction = "OUTGOING",
                                destination = "Claude Code",
                                mode = "streaming_with_tools",
                                chunk_type = "text_content",
                                chunk_json = %json,
                                "Sending synthetic text content chunk to Claude Code"
                            );
                        }
                    }

                    match serde_json::to_string(&text_chunk) {
                        Ok(json) => yield Ok(Event::default().data(json)),
                        Err(e) => {
                            tracing::warn!("Failed to serialize text content chunk: {e}");
                        }
                    }
                }

                // Emit final chunk with parsed tool_calls
                let tool_chunk = ChatCompletionChunk {
                    id: first.id.clone(),
                    object: first.object.clone(),
                    created: first.created,
                    model: first.model.clone(),
                    choices: vec![ChunkChoice {
                        index: 0,
                        delta: ChunkDelta {
                            role: None,
                            content: None,
                            tool_calls: Some(tool_calls),
                        },
                        finish_reason: Some("tool_calls".to_string()),
                    }],
                };

                // TRACE: Log synthetic tool_calls chunk being sent
                if tracing::enabled!(tracing::Level::TRACE) {
                    if let Ok(json) = serde_json::to_string(&tool_chunk) {
                        tracing::trace!(
                            direction = "OUTGOING",
                            destination = "Claude Code",
                            mode = "streaming_with_tools",
                            chunk_type = "tool_calls",
                            chunk_json = %json,
                            "Sending synthetic tool_calls chunk to Claude Code"
                        );
                    }
                }

                match serde_json::to_string(&tool_chunk) {
                    Ok(json) => yield Ok(Event::default().data(json)),
                    Err(e) => {
                        tracing::error!("Failed to serialize tool_calls chunk: {e}");
                        let err_json = serde_json::json!({
                            "error": { "message": format!("Failed to serialize tool calls: {e}"), "type": "stream_error" }
                        });
                        yield Ok(Event::default().data(err_json.to_string()));
                    }
                }
            }
        }
    };

    let done_stream = futures::stream::once(async {
        Ok::<Event, Infallible>(Event::default().data("[DONE]"))
    });
    let full_stream = event_stream.chain(done_stream);

    let sse = Sse::new(full_stream).keep_alive(
        KeepAlive::new().interval(Duration::from_secs(15)),
    );

    Ok(sse.into_response())
}
