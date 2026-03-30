use std::convert::Infallible;
use std::sync::Arc;
use std::time::Duration;

use axum::extract::State;
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use axum::Json;
use futures::StreamExt;

use crate::anthropic::types::{
    build_message_start_response, map_stop_reason, AnthropicRequest, ContentBlock,
    ContentBlockInput, ContentDelta, InputJsonDelta, MessageDeltaBody, MessageDeltaUsage,
    ResponseContentBlock, StreamEvent, TextDelta, ToolResultContent,
};
use crate::copilot::types::MessageContent;
use crate::error::AppError;
use crate::server::AppState;
use crate::tools::injector;
use crate::tools::parser;

/// Handler for `POST /v1/messages`.
///
/// Accepts Anthropic Messages API requests, translates them to OpenAI format,
/// forwards to the Copilot API, and translates the response back to Anthropic
/// format. Supports both streaming and non-streaming modes.
///
/// When the request contains Anthropic `tools`, they are translated to the
/// internal format, injected into the system prompt, and tool calls are parsed
/// from the model's text response.
pub async fn messages(
    State(state): State<Arc<AppState>>,
    Json(request): Json<AnthropicRequest>,
) -> Result<Response, AppError> {
    tracing::debug!(
        model = %request.model,
        stream = ?request.stream,
        num_messages = request.messages.len(),
        max_tokens = request.max_tokens,
        "Received Anthropic messages request"
    );

    // Validate: messages must be non-empty
    if request.messages.is_empty() {
        return Err(AppError::InvalidRequest(
            "messages must be a non-empty array".to_string(),
        ));
    }

    // Log if any messages contain tool results
    let has_tool_results_in_messages = request.messages.iter().any(|m| {
        matches!(&m.content, ContentBlockInput::Blocks(blocks)
            if blocks.iter().any(|b| matches!(b, ContentBlock::ToolResult { .. })))
    });

    if has_tool_results_in_messages {
        tracing::debug!("Request contains tool_result blocks - Claude Code is sending back tool execution results");

        // Log the tool results for debugging
        for (idx, msg) in request.messages.iter().enumerate() {
            if let ContentBlockInput::Blocks(blocks) = &msg.content {
                for block in blocks {
                    if let ContentBlock::ToolResult { tool_use_id, content, .. } = block {
                        let result_text = match content {
                            ToolResultContent::Text(s) => s.clone(),
                            ToolResultContent::Blocks(b) => format!("[{} blocks]", b.len()),
                        };
                        tracing::debug!(
                            message_index = idx,
                            tool_use_id = %tool_use_id,
                            result_preview = %result_text.chars().take(200).collect::<String>(),
                            "Tool result in message"
                        );
                    }
                }
            }
        }
    }

    let has_tools = request
        .tools
        .as_ref()
        .map_or(false, |t| !t.is_empty());

    // Translate Anthropic request to OpenAI format
    let mut openai_request = request.to_chat_completion_request();

    // Log model normalization if it happened
    if openai_request.model != request.model {
        tracing::info!(
            original_model = %request.model,
            normalized_model = %openai_request.model,
            "Model name normalized for GitHub Copilot compatibility"
        );
    }

    // Apply tool injection.
    // Convert Anthropic tool definitions to internal format and inject.
    // IMPORTANT: Only inject if tools are explicitly provided in this request.
    // Claude Code is responsible for re-sending tool definitions on every turn
    // when tool calling is active (including turns with tool_result blocks).
    if let Some(ref tools) = request.tools {
        if !tools.is_empty() {
            tracing::debug!(
                num_tools = tools.len(),
                tool_names = ?tools.iter().map(|t| &t.name).collect::<Vec<_>>(),
                "Injecting Anthropic tools into prompt"
            );
            let internal_tools: Vec<_> =
                tools.iter().map(|t| t.to_internal_tool()).collect();
            injector::inject_tools_into_messages(
                &mut openai_request.messages,
                &internal_tools,
            );
        }
    } else if has_tool_results_in_messages {
        // Tool results are present but no tool definitions were provided.
        // This is likely a bug in the client (Claude Code should re-send tool
        // definitions on every turn). Log a warning to help debug.
        tracing::warn!(
            "Request contains tool_result blocks but no tool definitions. \
             The model may generate malformed tool calls without schema context. \
             Claude Code should re-send tool definitions on every turn when tool \
             calling is active."
        );
    }

    // Translate tool-role messages (from tool_result blocks) into user messages.
    injector::translate_tool_messages(&mut openai_request.messages);

    // Strip tools/tool_choice — Copilot API does not accept them.
    openai_request.tools = None;
    openai_request.tool_choice = None;

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
        let parse_tools = has_tools;
        return handle_streaming(state, &copilot_token, &openai_request, parse_tools).await;
    }

    // Non-streaming: forward to the Copilot API and translate response
    let mut response = state
        .copilot_client
        .send_chat_completion(&copilot_token, &openai_request)
        .await?;

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
            "Raw response from Copilot (Anthropic endpoint)"
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
                    "Parsed tool calls from Anthropic response"
                );
                let stripped = parser::strip_tool_calls(&content_text);
                choice.message.content = if stripped.is_empty() {
                    MessageContent::Text(String::new())
                } else {
                    MessageContent::Text(stripped)
                };
                choice.message.tool_calls = Some(tool_calls);
                choice.finish_reason = Some("tool_calls".to_string());
            }
        }
    }

    let anthropic_response = response.to_anthropic_response();
    Ok(Json(anthropic_response).into_response())
}

/// Handle a streaming Anthropic Messages API request.
///
/// Translates OpenAI SSE chunks into Anthropic-format streaming events:
/// `message_start` → `content_block_start` → `content_block_delta`* →
/// `content_block_stop` → `message_delta` → `message_stop`.
///
/// When `parse_tools` is true, all chunks are buffered and tool calls are
/// detected at stream end. If tool calls are found, the text content is
/// stripped and `tool_use` content blocks are emitted with `stop_reason`
/// set to `"tool_use"`.
async fn handle_streaming(
    state: Arc<AppState>,
    copilot_token: &str,
    openai_request: &crate::copilot::types::ChatCompletionRequest,
    parse_tools: bool,
) -> Result<Response, AppError> {
    let chunk_stream = state
        .copilot_client
        .stream_chat_completion(copilot_token, openai_request)
        .await?;

    let model = openai_request.model.clone();

    if parse_tools {
        return handle_streaming_with_tools(chunk_stream, model).await;
    }

    // Normal streaming path (no tool parsing) — translate events inline.
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
                                content_block: ResponseContentBlock::text(String::new()),
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
                                delta: ContentDelta::Text(TextDelta {
                                    delta_type: "text_delta".to_string(),
                                    text: text.clone(),
                                }),
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
                    let err_json = serde_json::json!({
                        "type": "error",
                        "error": { "type": "api_error", "message": e.to_string() }
                    });
                    yield Ok::<Event, Infallible>(
                        Event::default().event("error").data(err_json.to_string())
                    );
                    return;
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

/// Handle streaming Anthropic responses when tool definitions are present.
///
/// Buffers all upstream chunks, detects tool calls in the accumulated text at
/// stream end, and emits proper Anthropic `tool_use` content blocks alongside
/// the stripped text content.
async fn handle_streaming_with_tools(
    chunk_stream: impl futures::Stream<
            Item = Result<crate::copilot::types::ChatCompletionChunk, AppError>,
        > + Send
        + 'static,
    model: String,
) -> Result<Response, AppError> {
    let event_stream = async_stream::stream! {
        let mut buffered_chunks: Vec<crate::copilot::types::ChatCompletionChunk> = Vec::new();
        let mut content_buffer = String::new();
        let mut stream = std::pin::pin!(chunk_stream);

        while let Some(result) = stream.next().await {
            match result {
                Ok(chunk) => {
                    for choice in &chunk.choices {
                        if let Some(ref text) = choice.delta.content {
                            content_buffer.push_str(text);
                        }
                    }
                    buffered_chunks.push(chunk);
                }
                Err(e) => {
                    tracing::error!(error = %e, "Error in upstream stream");
                    let err_json = serde_json::json!({
                        "type": "error",
                        "error": { "type": "api_error", "message": e.to_string() }
                    });
                    yield Ok::<Event, Infallible>(
                        Event::default().event("error").data(err_json.to_string())
                    );
                    return;
                }
            }
        }

        // Stream ended — check for tool calls
        tracing::debug!(
            content_length = content_buffer.len(),
            "Streaming response complete, checking for tool calls"
        );

        // Determine stream ID and model from buffered chunks
        let stream_id = buffered_chunks
            .first()
            .map(|c| c.id.clone())
            .unwrap_or_else(|| "msg_unknown".to_string());

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
                    "Full buffered content from streaming response"
                );
            } else {
                tracing::trace!(
                    content_preview = %content_buffer.chars().take(500).collect::<String>(),
                    content_length = content_buffer.len(),
                    "Buffered content preview (truncated)"
                );
            }
        } else {
            tracing::debug!(
                content_preview = %content_buffer.chars().take(200).collect::<String>(),
                "Buffered content preview"
            );
        }

        let tool_calls = parser::parse_tool_calls(&content_buffer);
        let has_tool_calls = !tool_calls.is_empty();

        if has_tool_calls {
            tracing::debug!(
                num_tool_calls = tool_calls.len(),
                tool_call_names = ?tool_calls.iter().map(|tc| &tc.function.name).collect::<Vec<_>>(),
                "Parsed tool calls from streaming response"
            );
        } else {
            tracing::debug!("No tool calls found in streaming response");
        }

        let stripped_text = if has_tool_calls {
            parser::strip_tool_calls(&content_buffer)
        } else {
            content_buffer.clone()
        };

        // === Emit message_start ===
        let msg = build_message_start_response(&stream_id, &model);
        let event = StreamEvent::MessageStart { message: msg };
        match serde_json::to_string(&event) {
            Ok(json) => yield Ok::<Event, Infallible>(
                Event::default().event("message_start").data(json)
            ),
            Err(e) => { tracing::error!("failed to serialise SSE event: {e}"); return; }
        }

        let mut block_index: u32 = 0;

        // === Emit text content block (if any text remains) ===
        if !stripped_text.is_empty() {
            // content_block_start for text
            let event = StreamEvent::ContentBlockStart {
                index: block_index,
                content_block: ResponseContentBlock::text(String::new()),
            };
            match serde_json::to_string(&event) {
                Ok(json) => yield Ok(Event::default().event("content_block_start").data(json)),
                Err(e) => { tracing::error!("failed to serialise SSE event: {e}"); return; }
            }

            // content_block_delta with full text
            let event = StreamEvent::ContentBlockDelta {
                index: block_index,
                delta: ContentDelta::Text(TextDelta {
                    delta_type: "text_delta".to_string(),
                    text: stripped_text,
                }),
            };
            match serde_json::to_string(&event) {
                Ok(json) => yield Ok(Event::default().event("content_block_delta").data(json)),
                Err(e) => { tracing::error!("failed to serialise SSE event: {e}"); return; }
            }

            // content_block_stop
            let event = StreamEvent::ContentBlockStop { index: block_index };
            match serde_json::to_string(&event) {
                Ok(json) => yield Ok(Event::default().event("content_block_stop").data(json)),
                Err(e) => { tracing::error!("failed to serialise SSE event: {e}"); return; }
            }

            block_index += 1;
        }

        // === Emit tool_use content blocks ===
        if has_tool_calls {
            for tc in &tool_calls {
                // content_block_start with tool_use (empty input initially)
                let tool_block = ResponseContentBlock::ToolUse {
                    block_type: "tool_use".to_string(),
                    id: tc.id.clone().unwrap_or_else(|| "call_unknown".to_string()),
                    name: tc.function.name.clone().unwrap_or_default(),
                    input: serde_json::Value::Object(serde_json::Map::new()),
                };
                let event = StreamEvent::ContentBlockStart {
                    index: block_index,
                    content_block: tool_block,
                };
                match serde_json::to_string(&event) {
                    Ok(json) => yield Ok(Event::default().event("content_block_start").data(json)),
                    Err(e) => { tracing::error!("failed to serialise SSE event: {e}"); return; }
                }

                // content_block_delta with input_json_delta containing full input
                let input_json = tc.function.arguments
                    .as_ref()
                    .cloned()
                    .unwrap_or_else(|| "{}".to_string());

                let event = StreamEvent::ContentBlockDelta {
                    index: block_index,
                    delta: ContentDelta::InputJson(InputJsonDelta {
                        delta_type: "input_json_delta".to_string(),
                        partial_json: input_json,
                    }),
                };
                match serde_json::to_string(&event) {
                    Ok(json) => yield Ok(Event::default().event("content_block_delta").data(json)),
                    Err(e) => { tracing::error!("failed to serialise SSE event: {e}"); return; }
                }

                // content_block_stop
                let event = StreamEvent::ContentBlockStop { index: block_index };
                match serde_json::to_string(&event) {
                    Ok(json) => yield Ok(Event::default().event("content_block_stop").data(json)),
                    Err(e) => { tracing::error!("failed to serialise SSE event: {e}"); return; }
                }

                block_index += 1;
            }
        }

        // === Emit message_delta with stop reason ===
        let stop_reason = if has_tool_calls {
            Some("tool_use".to_string())
        } else {
            // Find last finish_reason from buffered chunks
            let last_fr = buffered_chunks.iter().rev()
                .flat_map(|c| c.choices.iter())
                .find_map(|choice| choice.finish_reason.clone());
            map_stop_reason(last_fr.as_deref())
        };

        let event = StreamEvent::MessageDelta {
            delta: MessageDeltaBody {
                stop_reason,
                stop_sequence: None,
            },
            usage: MessageDeltaUsage { output_tokens: 0 },
        };
        match serde_json::to_string(&event) {
            Ok(json) => yield Ok(Event::default().event("message_delta").data(json)),
            Err(e) => { tracing::error!("failed to serialise SSE event: {e}"); return; }
        }

        // === Emit message_stop ===
        let event = StreamEvent::MessageStop {};
        match serde_json::to_string(&event) {
            Ok(json) => yield Ok(Event::default().event("message_stop").data(json)),
            Err(e) => { tracing::error!("failed to serialise SSE event: {e}"); return; }
        }
    };

    let sse = Sse::new(event_stream).keep_alive(
        KeepAlive::new().interval(Duration::from_secs(15)),
    );

    Ok(sse.into_response())
}
