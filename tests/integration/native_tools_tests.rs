//! Integration tests for native OpenAI tools handler integration (Epic 4).
//!
//! Tests the full request/response cycle through the `/v1/messages` endpoint
//! with `--native-tools` enabled, verifying:
//! - Non-streaming native tool call requests and responses (E4-T8)
//! - Streaming native tool call responses (E4-T9)
//! - Fallback to XML injection on unsupported error (E4-T10)
//! - Tool name truncation roundtrip (E4-T11)

use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::response::IntoResponse;
use axum::routing::post;
use axum::{Json, Router};
use serde_json::json;
use tokio::net::TcpListener;
use tower::ServiceExt;

use copilot_adapter::anthropic::types::AnthropicResponse;
use copilot_adapter::server::build_router;

use super::test_helpers::{create_test_state, create_test_state_native_tools};

// ---------------------------------------------------------------------------
// Mock servers
// ---------------------------------------------------------------------------

/// Spawn a mock GitHub server that provides Copilot tokens.
async fn spawn_mock_github() -> (std::net::SocketAddr, tokio::task::JoinHandle<()>) {
    let app = Router::new().route(
        "/copilot_internal/v2/token",
        axum::routing::get(|headers: axum::http::HeaderMap| async move {
            let auth = headers
                .get("Authorization")
                .and_then(|v| v.to_str().ok())
                .unwrap_or("");

            if auth == "token test_github_token" {
                let expires_at = chrono::Utc::now().timestamp() + 1800;
                Json(json!({
                    "token": "test_copilot_token",
                    "expires_at": expires_at
                }))
            } else {
                Json(json!({"error": "unauthorized"}))
            }
        }),
    );

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let handle = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });

    (addr, handle)
}

/// Spawn a mock Copilot API that returns a native tool call response
/// (non-streaming).
async fn spawn_mock_copilot_native_tool_call(
    tool_name: &str,
    args: serde_json::Value,
) -> (std::net::SocketAddr, tokio::task::JoinHandle<()>) {
    let tool_name = tool_name.to_string();
    let args_str = serde_json::to_string(&args).unwrap();

    let app = Router::new().route(
        "/chat/completions",
        post(move |headers: axum::http::HeaderMap, Json(body): Json<serde_json::Value>| {
            let tn = tool_name.clone();
            let a = args_str.clone();
            async move {
                // Validate required headers
                let auth = headers
                    .get("Authorization")
                    .and_then(|v| v.to_str().ok())
                    .unwrap_or("");
                if !auth.starts_with("Bearer ") {
                    return (StatusCode::UNAUTHORIZED, Json(json!({"error": "unauthorized"}))).into_response();
                }

                let model = body["model"].as_str().unwrap_or("gpt-4o");

                // Verify tools were forwarded in the request
                if body.get("tools").is_none() {
                    return (StatusCode::BAD_REQUEST, Json(json!({
                        "error": "Request should include tools field"
                    }))).into_response();
                }
                if body.get("tool_choice").is_none() {
                    return (StatusCode::BAD_REQUEST, Json(json!({
                        "error": "Request should include tool_choice"
                    }))).into_response();
                }

                Json(json!({
                    "id": "chatcmpl-native-e4",
                    "object": "chat.completion",
                    "created": 1700000000,
                    "model": model,
                    "choices": [{
                        "index": 0,
                        "message": {
                            "role": "assistant",
                            "content": serde_json::Value::Null,
                            "tool_calls": [{
                                "id": "call_e4_001",
                                "type": "function",
                                "function": {
                                    "name": tn,
                                    "arguments": a
                                }
                            }]
                        },
                        "finish_reason": "tool_calls"
                    }],
                    "usage": {
                        "prompt_tokens": 20,
                        "completion_tokens": 10,
                        "total_tokens": 30
                    }
                })).into_response()
            }
        }),
    );

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let handle = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });

    (addr, handle)
}

/// Spawn a mock Copilot API that returns a streaming native tool call response.
async fn spawn_mock_copilot_native_streaming_tool(
    tool_name: &str,
    args: serde_json::Value,
) -> (std::net::SocketAddr, tokio::task::JoinHandle<()>) {
    let tool_name = tool_name.to_string();
    let args = args.clone();

    let app = Router::new().route(
        "/chat/completions",
        post(move |_headers: axum::http::HeaderMap, Json(body): Json<serde_json::Value>| {
            let tn = tool_name.clone();
            let a = args.clone();
            async move {
                let model = body["model"].as_str().unwrap_or("gpt-4o").to_string();
                let stream = body["stream"].as_bool().unwrap_or(false);

                if !stream {
                    // Non-streaming fallback — shouldn't happen in streaming tests
                    return Json(json!({"error": "expected stream=true"})).into_response();
                }

                // Verify tools were forwarded
                if body.get("tools").is_none() {
                    return (StatusCode::BAD_REQUEST, Json(json!({
                        "error": "Request should include tools field"
                    }))).into_response();
                }

                let args_str = serde_json::to_string(&a).unwrap();
                let (frag1, frag2) = if args_str.len() > 10 {
                    let mid = args_str.len() / 2;
                    (args_str[..mid].to_string(), args_str[mid..].to_string())
                } else {
                    (args_str.clone(), String::new())
                };

                let mut chunks = vec![
                    // Role chunk
                    format!("data: {}\n\n", json!({
                        "id": "chatcmpl-e4-stream",
                        "object": "chat.completion.chunk",
                        "created": 1700000000,
                        "model": model,
                        "choices": [{"index": 0, "delta": {"role": "assistant"}, "finish_reason": serde_json::Value::Null}]
                    })),
                    // Tool call start
                    format!("data: {}\n\n", json!({
                        "id": "chatcmpl-e4-stream",
                        "object": "chat.completion.chunk",
                        "created": 1700000000,
                        "model": model,
                        "choices": [{"index": 0, "delta": {
                            "tool_calls": [{
                                "index": 0,
                                "id": "call_e4_stream_001",
                                "type": "function",
                                "function": {"name": tn, "arguments": ""}
                            }]
                        }, "finish_reason": serde_json::Value::Null}]
                    })),
                    // Arguments fragment 1
                    format!("data: {}\n\n", json!({
                        "id": "chatcmpl-e4-stream",
                        "object": "chat.completion.chunk",
                        "created": 1700000000,
                        "model": model,
                        "choices": [{"index": 0, "delta": {
                            "tool_calls": [{"index": 0, "function": {"arguments": frag1}}]
                        }, "finish_reason": serde_json::Value::Null}]
                    })),
                ];

                if !frag2.is_empty() {
                    chunks.push(format!("data: {}\n\n", json!({
                        "id": "chatcmpl-e4-stream",
                        "object": "chat.completion.chunk",
                        "created": 1700000000,
                        "model": model,
                        "choices": [{"index": 0, "delta": {
                            "tool_calls": [{"index": 0, "function": {"arguments": frag2}}]
                        }, "finish_reason": serde_json::Value::Null}]
                    })));
                }

                // Finish
                chunks.push(format!("data: {}\n\n", json!({
                    "id": "chatcmpl-e4-stream",
                    "object": "chat.completion.chunk",
                    "created": 1700000000,
                    "model": model,
                    "choices": [{"index": 0, "delta": {}, "finish_reason": "tool_calls"}]
                })));
                chunks.push("data: [DONE]\n\n".to_string());

                axum::http::Response::builder()
                    .status(200)
                    .header("Content-Type", "text/event-stream")
                    .body(Body::from(chunks.concat()))
                    .unwrap()
                    .into_response()
            }
        }),
    );

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let handle = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });

    (addr, handle)
}

/// Spawn a mock Copilot API that rejects tools with an error (simulates
/// "tools not supported" for fallback testing).
async fn spawn_mock_copilot_tools_not_supported() -> (std::net::SocketAddr, tokio::task::JoinHandle<()>) {
    let app = Router::new().route(
        "/chat/completions",
        post(|_headers: axum::http::HeaderMap, Json(body): Json<serde_json::Value>| async move {
            // If tools are present, reject with a 400 error
            if body.get("tools").is_some() {
                return (
                    StatusCode::BAD_REQUEST,
                    Json(json!({
                        "error": {
                            "message": "The 'tools' parameter is not supported for this model.",
                            "type": "invalid_request_error",
                            "code": "unsupported_parameter"
                        }
                    })),
                )
                    .into_response();
            }

            // No tools → respond with normal text (XML injection path)
            let model = body["model"].as_str().unwrap_or("gpt-4");
            Json(json!({
                "id": "chatcmpl-fallback",
                "object": "chat.completion",
                "created": 1700000000,
                "model": model,
                "choices": [{
                    "index": 0,
                    "message": {
                        "role": "assistant",
                        "content": "Hello from fallback path!"
                    },
                    "finish_reason": "stop"
                }],
                "usage": {
                    "prompt_tokens": 10,
                    "completion_tokens": 5,
                    "total_tokens": 15
                }
            }))
            .into_response()
        }),
    );

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let handle = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });

    (addr, handle)
}

/// Build a standard Anthropic messages request with tool definitions.
fn build_anthropic_request_with_tools(stream: bool) -> serde_json::Value {
    json!({
        "model": "claude-sonnet-4-20250514",
        "max_tokens": 1024,
        "stream": stream,
        "messages": [{
            "role": "user",
            "content": "What's the weather in London?"
        }],
        "tools": [{
            "name": "get_weather",
            "description": "Get weather for a location",
            "input_schema": {
                "type": "object",
                "properties": {
                    "location": { "type": "string", "description": "The city name" },
                    "units": { "type": "string", "enum": ["celsius", "fahrenheit"] }
                },
                "required": ["location"]
            }
        }]
    })
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Parse SSE events from a response body string into (event_type, data) pairs.
fn parse_sse_events(body_text: &str) -> Vec<(&str, &str)> {
    body_text
        .split("\n\n")
        .filter(|chunk| !chunk.trim().is_empty())
        .filter_map(|chunk| {
            let mut event_type = None;
            let mut data = None;
            for line in chunk.lines() {
                if let Some(rest) = line.strip_prefix("event: ") {
                    event_type = Some(rest.trim());
                } else if let Some(rest) = line.strip_prefix("data: ") {
                    data = Some(rest.trim());
                }
            }
            match (event_type, data) {
                (Some(e), Some(d)) => Some((e, d)),
                _ => None,
            }
        })
        .collect()
}

// ===========================================================================
// E4-T8: Non-streaming native tool call request/response
// ===========================================================================

#[tokio::test]
async fn native_tools_non_streaming_request_response() {
    let (github_addr, _gh) = spawn_mock_github().await;
    let (copilot_addr, _cp) =
        spawn_mock_copilot_native_tool_call("get_weather", json!({"location": "London"})).await;

    let state = create_test_state_native_tools(
        format!("http://{copilot_addr}/chat/completions"),
        github_addr,
    )
    .await;
    let app = build_router(state);

    let request_body = build_anthropic_request_with_tools(false);

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/messages")
                .header("Content-Type", "application/json")
                .body(Body::from(serde_json::to_string(&request_body).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);

    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let anthropic_response: AnthropicResponse = serde_json::from_slice(&body).unwrap();

    // Verify the response structure
    assert_eq!(anthropic_response.role, "assistant");
    assert_eq!(anthropic_response.stop_reason, Some("tool_use".to_string()));

    // Should contain a tool_use content block
    let tool_use_blocks: Vec<_> = anthropic_response
        .content
        .iter()
        .filter(|b| b.block_type() == "tool_use")
        .collect();
    assert_eq!(tool_use_blocks.len(), 1, "Expected exactly one tool_use block");

    if let copilot_adapter::anthropic::types::ResponseContentBlock::ToolUse {
        id, name, input, ..
    } = &tool_use_blocks[0]
    {
        assert_eq!(name, "get_weather");
        assert!(!id.is_empty());
        assert_eq!(input["location"], "London");
    } else {
        panic!("Expected ToolUse block");
    }
}

// ===========================================================================
// E4-T9: Streaming native tool call response
// ===========================================================================

#[tokio::test]
async fn native_tools_streaming_request_response() {
    let (github_addr, _gh) = spawn_mock_github().await;
    let (copilot_addr, _cp) =
        spawn_mock_copilot_native_streaming_tool("get_weather", json!({"location": "London"}))
            .await;

    let state = create_test_state_native_tools(
        format!("http://{copilot_addr}/chat/completions"),
        github_addr,
    )
    .await;
    let app = build_router(state);

    let request_body = build_anthropic_request_with_tools(true);

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/messages")
                .header("Content-Type", "application/json")
                .body(Body::from(serde_json::to_string(&request_body).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);

    // Parse SSE events from the streaming response body and validate ordering.
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let body_text = String::from_utf8(body.to_vec()).unwrap();

    let events = parse_sse_events(&body_text);

    // Validate we got events at all
    assert!(
        !events.is_empty(),
        "Should have parsed SSE events from response"
    );

    // Extract just the event types for ordering validation
    let event_types: Vec<&str> = events.iter().map(|(t, _)| *t).collect();

    // Validate required event types are present
    assert!(
        event_types.contains(&"message_start"),
        "Missing message_start event"
    );
    assert!(
        event_types.contains(&"content_block_start"),
        "Missing content_block_start event"
    );
    assert!(
        event_types.contains(&"content_block_delta"),
        "Missing content_block_delta event"
    );
    assert!(
        event_types.contains(&"content_block_stop"),
        "Missing content_block_stop event"
    );
    assert!(
        event_types.contains(&"message_delta"),
        "Missing message_delta event"
    );
    assert!(
        event_types.contains(&"message_stop"),
        "Missing message_stop event"
    );

    // Validate ordering: message_start must come first
    let msg_start_pos = event_types.iter().position(|&t| t == "message_start").unwrap();
    assert_eq!(msg_start_pos, 0, "message_start must be the first event");

    // Validate ordering: message_stop must come last
    let msg_stop_pos = event_types.iter().rposition(|&t| t == "message_stop").unwrap();
    assert_eq!(
        msg_stop_pos,
        event_types.len() - 1,
        "message_stop must be the last event"
    );

    // Validate ordering: message_delta must come after all content_block_stop events
    // and before message_stop
    let msg_delta_pos = event_types.iter().position(|&t| t == "message_delta").unwrap();
    let last_block_stop_pos = event_types
        .iter()
        .rposition(|&t| t == "content_block_stop")
        .unwrap();
    assert!(
        msg_delta_pos > last_block_stop_pos,
        "message_delta (pos {msg_delta_pos}) must come after last content_block_stop (pos {last_block_stop_pos})"
    );
    assert!(
        msg_delta_pos < msg_stop_pos,
        "message_delta (pos {msg_delta_pos}) must come before message_stop (pos {msg_stop_pos})"
    );

    // Validate ordering: content_block_start must come before its corresponding
    // content_block_delta and content_block_stop
    let first_block_start_pos = event_types.iter().position(|&t| t == "content_block_start").unwrap();
    let first_block_delta_pos = event_types.iter().position(|&t| t == "content_block_delta").unwrap();
    let first_block_stop_pos = event_types.iter().position(|&t| t == "content_block_stop").unwrap();
    assert!(
        first_block_start_pos < first_block_delta_pos,
        "content_block_start must come before content_block_delta"
    );
    assert!(
        first_block_delta_pos < first_block_stop_pos,
        "content_block_delta must come before content_block_stop"
    );

    // Validate content: find tool_use content_block_start and verify it has the right type
    let tool_use_start = events.iter().find(|(event_type, data)| {
        *event_type == "content_block_start" && {
            serde_json::from_str::<serde_json::Value>(data)
                .ok()
                .and_then(|v| v.get("content_block")?.get("type")?.as_str().map(|s| s == "tool_use"))
                .unwrap_or(false)
        }
    });
    assert!(tool_use_start.is_some(), "Should contain tool_use content_block_start");

    // Verify the tool name in the tool_use block
    let (_, tool_start_data) = tool_use_start.unwrap();
    let tool_start_json: serde_json::Value = serde_json::from_str(tool_start_data).unwrap();
    assert_eq!(
        tool_start_json["content_block"]["name"], "get_weather",
        "Tool use block should contain get_weather"
    );

    // Should contain input_json_delta
    let has_input_json_delta = events.iter().any(|(event_type, data)| {
        *event_type == "content_block_delta" && {
            serde_json::from_str::<serde_json::Value>(data)
                .ok()
                .and_then(|v| v.get("delta")?.get("type")?.as_str().map(|s| s == "input_json_delta"))
                .unwrap_or(false)
        }
    });
    assert!(has_input_json_delta, "Should contain input_json_delta");

    // The stop_reason in message_delta should be tool_use
    let message_delta_event = events.iter().find(|(event_type, _)| *event_type == "message_delta");
    assert!(message_delta_event.is_some(), "Should have a message_delta event");
    let (_, delta_data) = message_delta_event.unwrap();
    let delta_json: serde_json::Value = serde_json::from_str(delta_data).unwrap();
    assert_eq!(
        delta_json["delta"]["stop_reason"], "tool_use",
        "message_delta stop_reason should be tool_use"
    );
}

// ===========================================================================
// E4-T10: Fallback to XML injection
// ===========================================================================

#[tokio::test]
async fn native_tools_fallback_to_xml_on_unsupported() {
    let (github_addr, _gh) = spawn_mock_github().await;
    let (copilot_addr, _cp) = spawn_mock_copilot_tools_not_supported().await;

    let state = create_test_state_native_tools(
        format!("http://{copilot_addr}/chat/completions"),
        github_addr,
    )
    .await;
    let app = build_router(state);

    let request_body = build_anthropic_request_with_tools(false);

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/messages")
                .header("Content-Type", "application/json")
                .body(Body::from(serde_json::to_string(&request_body).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();

    // The fallback to XML injection should succeed — the second request
    // (without tools field) should return a normal text response.
    assert_eq!(response.status(), StatusCode::OK);

    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let anthropic_response: AnthropicResponse = serde_json::from_slice(&body).unwrap();

    assert_eq!(anthropic_response.role, "assistant");
    // The fallback path returns a normal text response
    assert!(
        anthropic_response.content.iter().any(|b| {
            if let copilot_adapter::anthropic::types::ResponseContentBlock::Text { text, .. } = b {
                text.contains("Hello from fallback path!")
            } else {
                false
            }
        }),
        "Fallback response should contain text from XML injection path"
    );
}

// ===========================================================================
// E4-T10 (continued): Fallback on double-quoted error messages
// ===========================================================================

/// Spawn a mock Copilot API that rejects tools with a double-quoted error
/// message (e.g. `"tools" is not supported`), verifying the double-quote
/// branches in `is_tools_not_supported_error`.
///
/// The mock deliberately uses a `type` and omits `code` so that only the
/// double-quoted branch (`"\"tools\"" && "not supported"`) in
/// `is_tools_not_supported_error` can match. Previous versions used
/// `"code": "unsupported_parameter"`, which matched via the broader
/// `contains("unsupported_parameter")` branch, silently bypassing the
/// double-quoted detection logic.
async fn spawn_mock_copilot_tools_not_supported_double_quoted() -> (std::net::SocketAddr, tokio::task::JoinHandle<()>) {
    let app = Router::new().route(
        "/chat/completions",
        post(|_headers: axum::http::HeaderMap, Json(body): Json<serde_json::Value>| async move {
            if body.get("tools").is_some() {
                return (
                    StatusCode::BAD_REQUEST,
                    Json(json!({
                        "error": {
                            "message": "The \"tools\" parameter is not supported for this model.",
                            "type": "unsupported_feature"
                        }
                    })),
                )
                    .into_response();
            }

            let model = body["model"].as_str().unwrap_or("gpt-4");
            Json(json!({
                "id": "chatcmpl-fallback-dq",
                "object": "chat.completion",
                "created": 1700000000,
                "model": model,
                "choices": [{
                    "index": 0,
                    "message": {
                        "role": "assistant",
                        "content": "Hello from double-quoted fallback!"
                    },
                    "finish_reason": "stop"
                }],
                "usage": {
                    "prompt_tokens": 10,
                    "completion_tokens": 5,
                    "total_tokens": 15
                }
            }))
            .into_response()
        }),
    );

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let handle = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });

    (addr, handle)
}

#[tokio::test]
async fn native_tools_fallback_on_double_quoted_error() {
    let (github_addr, _gh) = spawn_mock_github().await;
    let (copilot_addr, _cp) = spawn_mock_copilot_tools_not_supported_double_quoted().await;

    let state = create_test_state_native_tools(
        format!("http://{copilot_addr}/chat/completions"),
        github_addr,
    )
    .await;
    let app = build_router(state);

    let request_body = build_anthropic_request_with_tools(false);

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/messages")
                .header("Content-Type", "application/json")
                .body(Body::from(serde_json::to_string(&request_body).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();

    // The fallback should succeed — double-quoted error triggers XML fallback.
    assert_eq!(response.status(), StatusCode::OK);

    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let anthropic_response: AnthropicResponse = serde_json::from_slice(&body).unwrap();

    assert_eq!(anthropic_response.role, "assistant");
    assert!(
        anthropic_response.content.iter().any(|b| {
            if let copilot_adapter::anthropic::types::ResponseContentBlock::Text { text, .. } = b {
                text.contains("Hello from double-quoted fallback!")
            } else {
                false
            }
        }),
        "Double-quoted error should trigger fallback to XML injection path"
    );
}

// ===========================================================================
// E4-T10 (continued): native_tools=false bypasses native path
// ===========================================================================

#[tokio::test]
async fn without_native_tools_flag_uses_xml_injection() {
    let (github_addr, _gh) = spawn_mock_github().await;
    let (copilot_addr, _cp) = spawn_mock_copilot_tools_not_supported().await;

    // Use default config (native_tools: false)
    let state = create_test_state(
        format!("http://{copilot_addr}/chat/completions"),
        github_addr,
    )
    .await;
    let app = build_router(state);

    let request_body = build_anthropic_request_with_tools(false);

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/messages")
                .header("Content-Type", "application/json")
                .body(Body::from(serde_json::to_string(&request_body).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();

    // Even though the mock rejects tools, the XML injection path doesn't
    // send tools in the request, so it should succeed.
    assert_eq!(response.status(), StatusCode::OK);

    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let anthropic_response: AnthropicResponse = serde_json::from_slice(&body).unwrap();

    assert_eq!(anthropic_response.role, "assistant");
    assert!(
        anthropic_response.content.iter().any(|b| {
            if let copilot_adapter::anthropic::types::ResponseContentBlock::Text { text, .. } = b {
                text.contains("Hello from fallback path!")
            } else {
                false
            }
        }),
        "XML injection path should return text response"
    );
}

// ===========================================================================
// E4-T11: Tool name truncation roundtrip
// ===========================================================================

#[tokio::test]
async fn native_tools_name_truncation_roundtrip() {
    // Create a tool name longer than 64 characters
    let long_tool_name = "this_is_a_very_long_tool_name_that_exceeds_the_sixty_four_character_limit_for_openai";
    assert!(
        long_tool_name.len() > 64,
        "Test tool name must exceed 64 chars"
    );

    // The mock server will echo back whatever truncated name it receives
    // (simulating the Copilot API using the truncated name).
    let (github_addr, _gh) = spawn_mock_github().await;

    // Spawn a mock that captures and echoes the received tool name
    let app = Router::new().route(
        "/chat/completions",
        post(|_headers: axum::http::HeaderMap, Json(body): Json<serde_json::Value>| async move {
            let model = body["model"].as_str().unwrap_or("gpt-4o");

            // Extract the (potentially truncated) tool name from the request
            let tools = match body["tools"].as_array() {
                Some(t) => t,
                None => {
                    return (StatusCode::BAD_REQUEST, Json(json!({
                        "error": "Request should include tools array"
                    }))).into_response();
                }
            };
            let received_name = tools[0]["function"]["name"].as_str().unwrap().to_string();

            // Verify the name was truncated
            if received_name.len() > 64 {
                return (StatusCode::BAD_REQUEST, Json(json!({
                    "error": format!("Tool name should be truncated to 64 chars, got {} chars", received_name.len())
                }))).into_response();
            }

            // Echo the truncated name back in the tool_calls response
            Json(json!({
                "id": "chatcmpl-truncation-test",
                "object": "chat.completion",
                "created": 1700000000,
                "model": model,
                "choices": [{
                    "index": 0,
                    "message": {
                        "role": "assistant",
                        "content": serde_json::Value::Null,
                        "tool_calls": [{
                            "id": "call_trunc_001",
                            "type": "function",
                            "function": {
                                "name": received_name,
                                "arguments": "{\"test\": true}"
                            }
                        }]
                    },
                    "finish_reason": "tool_calls"
                }],
                "usage": {
                    "prompt_tokens": 20,
                    "completion_tokens": 10,
                    "total_tokens": 30
                }
            }))
            .into_response()
        }),
    );

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let copilot_addr = listener.local_addr().unwrap();
    let _cp = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });

    let state = create_test_state_native_tools(
        format!("http://{copilot_addr}/chat/completions"),
        github_addr,
    )
    .await;
    let router = build_router(state);

    let request_body = json!({
        "model": "claude-sonnet-4-20250514",
        "max_tokens": 1024,
        "messages": [{
            "role": "user",
            "content": "Test tool name truncation"
        }],
        "tools": [{
            "name": long_tool_name,
            "description": "A tool with a very long name",
            "input_schema": {
                "type": "object",
                "properties": {
                    "test": { "type": "boolean" }
                }
            }
        }]
    });

    let response = router
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/messages")
                .header("Content-Type", "application/json")
                .body(Body::from(serde_json::to_string(&request_body).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);

    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let anthropic_response: AnthropicResponse = serde_json::from_slice(&body).unwrap();

    // The response should have the ORIGINAL (non-truncated) name restored
    let tool_use_blocks: Vec<_> = anthropic_response
        .content
        .iter()
        .filter(|b| b.block_type() == "tool_use")
        .collect();
    assert_eq!(tool_use_blocks.len(), 1);

    if let copilot_adapter::anthropic::types::ResponseContentBlock::ToolUse {
        name, ..
    } = &tool_use_blocks[0]
    {
        assert_eq!(
            name, long_tool_name,
            "Tool name should be restored to the original long name, got: {name}"
        );
    } else {
        panic!("Expected ToolUse block");
    }
}

// ===========================================================================
// E4-T9 (additional): Streaming with text + tool call
// ===========================================================================

#[tokio::test]
async fn native_tools_streaming_text_then_tool_call() {
    let (github_addr, _gh) = spawn_mock_github().await;

    // Spawn a mock that returns text followed by a tool call
    let app = Router::new().route(
        "/chat/completions",
        post(|_headers: axum::http::HeaderMap, Json(body): Json<serde_json::Value>| async move {
            let model = body["model"].as_str().unwrap_or("gpt-4o").to_string();

            let chunks = vec![
                format!("data: {}\n\n", json!({
                    "id": "chatcmpl-e4-mixed",
                    "object": "chat.completion.chunk",
                    "created": 1700000000,
                    "model": model,
                    "choices": [{"index": 0, "delta": {"role": "assistant"}, "finish_reason": serde_json::Value::Null}]
                })),
                format!("data: {}\n\n", json!({
                    "id": "chatcmpl-e4-mixed",
                    "object": "chat.completion.chunk",
                    "created": 1700000000,
                    "model": model,
                    "choices": [{"index": 0, "delta": {"content": "Let me check the weather."}, "finish_reason": serde_json::Value::Null}]
                })),
                format!("data: {}\n\n", json!({
                    "id": "chatcmpl-e4-mixed",
                    "object": "chat.completion.chunk",
                    "created": 1700000000,
                    "model": model,
                    "choices": [{"index": 0, "delta": {
                        "tool_calls": [{
                            "index": 0,
                            "id": "call_mixed_001",
                            "type": "function",
                            "function": {"name": "get_weather", "arguments": ""}
                        }]
                    }, "finish_reason": serde_json::Value::Null}]
                })),
                format!("data: {}\n\n", json!({
                    "id": "chatcmpl-e4-mixed",
                    "object": "chat.completion.chunk",
                    "created": 1700000000,
                    "model": model,
                    "choices": [{"index": 0, "delta": {
                        "tool_calls": [{"index": 0, "function": {"arguments": "{\"location\":\"London\"}"}}]
                    }, "finish_reason": serde_json::Value::Null}]
                })),
                format!("data: {}\n\n", json!({
                    "id": "chatcmpl-e4-mixed",
                    "object": "chat.completion.chunk",
                    "created": 1700000000,
                    "model": model,
                    "choices": [{"index": 0, "delta": {}, "finish_reason": "tool_calls"}]
                })),
                "data: [DONE]\n\n".to_string(),
            ];

            axum::http::Response::builder()
                .status(200)
                .header("Content-Type", "text/event-stream")
                .body(Body::from(chunks.concat()))
                .unwrap()
                .into_response()
        }),
    );

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let copilot_addr = listener.local_addr().unwrap();
    let _cp = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });

    let state = create_test_state_native_tools(
        format!("http://{copilot_addr}/chat/completions"),
        github_addr,
    )
    .await;
    let router = build_router(state);

    let request_body = build_anthropic_request_with_tools(true);

    let response = router
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/messages")
                .header("Content-Type", "application/json")
                .body(Body::from(serde_json::to_string(&request_body).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);

    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let body_text = String::from_utf8(body.to_vec()).unwrap();

    let events = parse_sse_events(&body_text);

    assert!(!events.is_empty(), "Should have parsed SSE events");

    let event_types: Vec<&str> = events.iter().map(|(t, _)| *t).collect();

    // Validate ordering: message_start must be first, message_stop must be last
    assert_eq!(event_types[0], "message_start", "message_start must be first");
    assert_eq!(event_types[event_types.len() - 1], "message_stop", "message_stop must be last");

    // Validate text content block appears before tool_use content block.
    // Find the content_block_start events and check their types.
    let block_starts: Vec<(usize, serde_json::Value)> = events
        .iter()
        .enumerate()
        .filter(|(_, (t, _))| *t == "content_block_start")
        .map(|(pos, (_, data))| (pos, serde_json::from_str::<serde_json::Value>(data).unwrap()))
        .collect();

    assert!(
        block_starts.len() >= 2,
        "Should have at least 2 content_block_start events (text + tool_use), got {}",
        block_starts.len()
    );

    // First block should be text
    assert_eq!(
        block_starts[0].1["content_block"]["type"], "text",
        "First content block should be text"
    );

    // Second block should be tool_use
    assert_eq!(
        block_starts[1].1["content_block"]["type"], "tool_use",
        "Second content block should be tool_use"
    );
    assert_eq!(
        block_starts[1].1["content_block"]["name"], "get_weather",
        "Tool use block should be named get_weather"
    );

    // Text block must come before tool_use block
    assert!(
        block_starts[0].0 < block_starts[1].0,
        "Text content block must come before tool_use content block"
    );

    // Verify text content contains the expected text
    let text_deltas: Vec<&str> = events
        .iter()
        .filter(|(t, _)| *t == "content_block_delta")
        .filter_map(|(_, data)| {
            let v = serde_json::from_str::<serde_json::Value>(data).ok()?;
            if v["delta"]["type"].as_str()? == "text_delta" {
                Some(*data)
            } else {
                None
            }
        })
        .collect();
    assert!(!text_deltas.is_empty(), "Should have text_delta events");
    let has_weather_text = text_deltas.iter().any(|d| d.contains("Let me check the weather"));
    assert!(has_weather_text, "Text deltas should contain 'Let me check the weather'");

    // The stop_reason in message_delta should be tool_use
    let message_delta_event = events.iter().find(|(t, _)| *t == "message_delta");
    assert!(message_delta_event.is_some(), "Should have a message_delta event");
    let (_, delta_data) = message_delta_event.unwrap();
    let delta_json: serde_json::Value = serde_json::from_str(delta_data).unwrap();
    assert_eq!(
        delta_json["delta"]["stop_reason"], "tool_use",
        "message_delta stop_reason should be tool_use"
    );
}
