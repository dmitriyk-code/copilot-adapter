//! Epic 5 Task 5.7: Integration tests for streaming truncation.
//!
//! Verifies that when the Copilot API returns SSE chunks ending with
//! `finish_reason: "length"` during a tool call, the adapter emits a
//! truncation notice text block instead of a tool_use block.

use std::sync::Arc;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::response::IntoResponse;
use axum::routing::post;
use axum::Router;
use serde_json::json;
use tokio::net::TcpListener;
use tower::ServiceExt;

use copilot_adapter::auth::device_flow::DeviceFlowAuth;
use copilot_adapter::auth::token::TokenManager;
use copilot_adapter::copilot::client::CopilotClient;
use copilot_adapter::copilot::models_cache::ModelsCache;
use copilot_adapter::server::{build_router, AdapterConfig, AppState};

use super::test_helpers::InMemoryStorage;

/// Spawn a mock GitHub server that returns Copilot tokens.
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
                axum::Json(json!({
                    "token": "test_copilot_token",
                    "expires_at": expires_at
                }))
            } else {
                axum::Json(json!({"error": "unauthorized"}))
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

/// Spawn a mock Copilot API that returns a streaming response with a tool call
/// that gets truncated by `finish_reason: "length"`.
async fn spawn_mock_copilot_truncated_tool() -> (std::net::SocketAddr, tokio::task::JoinHandle<()>)
{
    let app = Router::new().route(
        "/chat/completions",
        post(|| async {
            // SSE chunks: tool call start, tool call args, then length truncation
            let chunks = vec![
                // Chunk 1: role announcement
                format!(
                    "data: {}\n\n",
                    json!({
                        "id": "chatcmpl-trunc",
                        "object": "chat.completion.chunk",
                        "created": 1700000000,
                        "model": "claude-sonnet-4.5",
                        "choices": [{"index": 0, "delta": {"role": "assistant"}, "finish_reason": null}]
                    })
                ),
                // Chunk 2: tool call start with name and first args
                format!(
                    "data: {}\n\n",
                    json!({
                        "id": "chatcmpl-trunc",
                        "object": "chat.completion.chunk",
                        "created": 1700000000,
                        "model": "claude-sonnet-4.5",
                        "choices": [{
                            "index": 0,
                            "delta": {
                                "tool_calls": [{
                                    "index": 0,
                                    "id": "call_abc123",
                                    "type": "function",
                                    "function": {
                                        "name": "Write",
                                        "arguments": "{\"file_path\": \"test.md\", \"content\": \"He"
                                    }
                                }]
                            },
                            "finish_reason": null
                        }]
                    })
                ),
                // Chunk 3: more tool call args
                format!(
                    "data: {}\n\n",
                    json!({
                        "id": "chatcmpl-trunc",
                        "object": "chat.completion.chunk",
                        "created": 1700000000,
                        "model": "claude-sonnet-4.5",
                        "choices": [{
                            "index": 0,
                            "delta": {
                                "tool_calls": [{
                                    "index": 0,
                                    "function": {
                                        "arguments": "llo world, this is a very long c"
                                    }
                                }]
                            },
                            "finish_reason": null
                        }]
                    })
                ),
                // Chunk 4: finish_reason="length" — truncation!
                format!(
                    "data: {}\n\n",
                    json!({
                        "id": "chatcmpl-trunc",
                        "object": "chat.completion.chunk",
                        "created": 1700000000,
                        "model": "claude-sonnet-4.5",
                        "choices": [{
                            "index": 0,
                            "delta": {},
                            "finish_reason": "length"
                        }]
                    })
                ),
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
    let addr = listener.local_addr().unwrap();
    let handle = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    (addr, handle)
}

/// Helper to parse Anthropic SSE events from response body.
fn parse_anthropic_sse(body_text: &str) -> Vec<(String, serde_json::Value)> {
    let mut events = Vec::new();
    let mut current_event_type = String::new();

    for line in body_text.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        if let Some(event_type) = line.strip_prefix("event:") {
            current_event_type = event_type.trim().to_string();
        } else if let Some(data) = line.strip_prefix("data:") {
            let data = data.trim();
            if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(data) {
                events.push((current_event_type.clone(), parsed));
            }
        }
    }

    events
}

#[tokio::test]
async fn streaming_truncated_tool_emits_text_notice_not_tool_use() {
    let (copilot_addr, _h1) = spawn_mock_copilot_truncated_tool().await;
    let (github_addr, _h2) = spawn_mock_github().await;

    let auth = DeviceFlowAuth::with_urls(
        format!("http://{github_addr}/unused"),
        format!("http://{github_addr}/unused"),
        format!("http://{github_addr}/copilot_internal/v2/token"),
    );
    let storage = InMemoryStorage::with_token("test_github_token");
    let tm = Arc::new(TokenManager::new(Box::new(storage), auth).await.unwrap());
    let client = reqwest::Client::new();

    let config = AdapterConfig {
        native_tools: true,
        ..AdapterConfig::default()
    };

    let state = Arc::new(AppState {
        token_manager: tm,
        copilot_client: CopilotClient::with_api_url(
            client,
            format!("http://{copilot_addr}/chat/completions"),
        ),
        config,
        models_cache: ModelsCache::new(std::time::Duration::from_secs(300)),
        conversation_logger: None,
    });
    let app = build_router(state);

    let body = json!({
        "model": "claude-sonnet-4-20250514",
        "max_tokens": 8192,
        "messages": [{"role": "user", "content": "Write a large file"}],
        "stream": true,
        "tools": [{
            "name": "Write",
            "description": "Write file",
            "input_schema": {
                "type": "object",
                "properties": {
                    "file_path": {"type": "string"},
                    "content": {"type": "string"}
                }
            }
        }]
    });

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/messages")
                .header("Content-Type", "application/json")
                .body(Body::from(serde_json::to_vec(&body).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);

    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let body_text = String::from_utf8_lossy(&bytes);
    let events = parse_anthropic_sse(&body_text);

    // Verify NO tool_use blocks in the output
    let tool_use_blocks: Vec<_> = events
        .iter()
        .filter(|(t, v)| {
            t == "content_block_start"
                && v.get("content_block")
                    .and_then(|cb| cb.get("type"))
                    .and_then(|t| t.as_str())
                    == Some("tool_use")
        })
        .collect();
    assert!(
        tool_use_blocks.is_empty(),
        "Should NOT contain any tool_use blocks, got: {tool_use_blocks:?}"
    );

    // Verify there IS a text content block with the truncation notice
    let text_deltas: Vec<_> = events
        .iter()
        .filter(|(t, v)| {
            t == "content_block_delta"
                && v.get("delta")
                    .and_then(|d| d.get("text"))
                    .and_then(|t| t.as_str())
                    .map_or(false, |text| text.contains("truncated"))
        })
        .collect();
    assert!(
        !text_deltas.is_empty(),
        "Should contain a truncation notice text delta"
    );

    // Verify there IS a content_block_start with type: "text" for the notice
    let text_blocks: Vec<_> = events
        .iter()
        .filter(|(t, v)| {
            t == "content_block_start"
                && v.get("content_block")
                    .and_then(|cb| cb.get("type"))
                    .and_then(|t| t.as_str())
                    == Some("text")
        })
        .collect();
    assert!(
        !text_blocks.is_empty(),
        "Should contain at least one text content block start"
    );

    // Verify message_delta has stop_reason "max_tokens"
    let msg_delta = events
        .iter()
        .find(|(t, _)| t == "message_delta")
        .expect("should have message_delta event");
    assert_eq!(
        msg_delta.1["delta"]["stop_reason"], "max_tokens",
        "stop_reason should be max_tokens for truncated tool call"
    );
}

// ===========================================================================
// Epic 6-T4: Integration tests for streaming usage fields
// ===========================================================================

/// Spawn a mock Copilot API that returns a simple text streaming response
/// (no upstream `usage` in chunks).
async fn spawn_mock_copilot_text_response() -> (std::net::SocketAddr, tokio::task::JoinHandle<()>) {
    let app = Router::new().route(
        "/chat/completions",
        post(|| async {
            let chunks = vec![
                format!(
                    "data: {}\n\n",
                    json!({
                        "id": "chatcmpl-usage-test",
                        "object": "chat.completion.chunk",
                        "created": 1700000000,
                        "model": "claude-sonnet-4.5",
                        "choices": [{"index": 0, "delta": {"role": "assistant"}, "finish_reason": null}]
                    })
                ),
                format!(
                    "data: {}\n\n",
                    json!({
                        "id": "chatcmpl-usage-test",
                        "object": "chat.completion.chunk",
                        "created": 1700000000,
                        "model": "claude-sonnet-4.5",
                        "choices": [{"index": 0, "delta": {"content": "Hello from the adapter!"}, "finish_reason": null}]
                    })
                ),
                format!(
                    "data: {}\n\n",
                    json!({
                        "id": "chatcmpl-usage-test",
                        "object": "chat.completion.chunk",
                        "created": 1700000000,
                        "model": "claude-sonnet-4.5",
                        "choices": [{"index": 0, "delta": {}, "finish_reason": "stop"}]
                    })
                ),
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
    let addr = listener.local_addr().unwrap();
    let handle = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    (addr, handle)
}

/// Spawn a mock Copilot API that includes upstream `usage` in the final chunk.
async fn spawn_mock_copilot_with_usage() -> (std::net::SocketAddr, tokio::task::JoinHandle<()>) {
    let app = Router::new().route(
        "/chat/completions",
        post(|| async {
            let chunks = vec![
                format!(
                    "data: {}\n\n",
                    json!({
                        "id": "chatcmpl-usage-override",
                        "object": "chat.completion.chunk",
                        "created": 1700000000,
                        "model": "claude-sonnet-4.5",
                        "choices": [{"index": 0, "delta": {"role": "assistant"}, "finish_reason": null}]
                    })
                ),
                format!(
                    "data: {}\n\n",
                    json!({
                        "id": "chatcmpl-usage-override",
                        "object": "chat.completion.chunk",
                        "created": 1700000000,
                        "model": "claude-sonnet-4.5",
                        "choices": [{"index": 0, "delta": {"content": "Response text"}, "finish_reason": null}]
                    })
                ),
                // Usage arrives before finish_reason
                format!(
                    "data: {}\n\n",
                    json!({
                        "id": "chatcmpl-usage-override",
                        "object": "chat.completion.chunk",
                        "created": 1700000000,
                        "model": "claude-sonnet-4.5",
                        "choices": [],
                        "usage": {"prompt_tokens": 999, "completion_tokens": 888, "total_tokens": 1887}
                    })
                ),
                format!(
                    "data: {}\n\n",
                    json!({
                        "id": "chatcmpl-usage-override",
                        "object": "chat.completion.chunk",
                        "created": 1700000000,
                        "model": "claude-sonnet-4.5",
                        "choices": [{"index": 0, "delta": {}, "finish_reason": "stop"}]
                    })
                ),
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
    let addr = listener.local_addr().unwrap();
    let handle = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    (addr, handle)
}

/// Helper to build a standard streaming request body for integration tests.
fn streaming_request_body() -> serde_json::Value {
    json!({
        "model": "claude-sonnet-4-20250514",
        "max_tokens": 8192,
        "messages": [{"role": "user", "content": "Hello, how are you?"}],
        "stream": true
    })
}

/// Helper to send a streaming request and collect parsed SSE events.
async fn send_streaming_request(
    app: Router,
    body: &serde_json::Value,
) -> Vec<(String, serde_json::Value)> {
    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/messages")
                .header("Content-Type", "application/json")
                .body(Body::from(serde_json::to_vec(body).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);

    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let body_text = String::from_utf8_lossy(&bytes);
    parse_anthropic_sse(&body_text)
}

/// `message_start.message.usage.input_tokens > 0` when the Copilot API
/// returns a simple text response with no upstream usage data.
#[tokio::test]
async fn streaming_response_has_nonzero_input_tokens() {
    let (copilot_addr, _h1) = spawn_mock_copilot_text_response().await;
    let (github_addr, _h2) = spawn_mock_github().await;

    let state =
        super::test_helpers::create_test_state(format!("http://{copilot_addr}/chat/completions"), github_addr)
            .await;
    let app = build_router(state);

    let events = send_streaming_request(app, &streaming_request_body()).await;

    let msg_start = events
        .iter()
        .find(|(t, _)| t == "message_start")
        .expect("should have message_start event");

    let input_tokens = msg_start.1["message"]["usage"]["input_tokens"]
        .as_u64()
        .expect("input_tokens should be a number");
    assert!(
        input_tokens > 0,
        "input_tokens should be > 0, got {input_tokens}"
    );
}

/// `message_delta.usage.output_tokens > 0` when the Copilot API returns
/// text content (no upstream usage data).
#[tokio::test]
async fn streaming_response_has_nonzero_output_tokens() {
    let (copilot_addr, _h1) = spawn_mock_copilot_text_response().await;
    let (github_addr, _h2) = spawn_mock_github().await;

    let state =
        super::test_helpers::create_test_state(format!("http://{copilot_addr}/chat/completions"), github_addr)
            .await;
    let app = build_router(state);

    let events = send_streaming_request(app, &streaming_request_body()).await;

    let msg_delta = events
        .iter()
        .find(|(t, _)| t == "message_delta")
        .expect("should have message_delta event");

    let output_tokens = msg_delta.1["usage"]["output_tokens"]
        .as_u64()
        .expect("output_tokens should be a number");
    assert!(
        output_tokens > 0,
        "output_tokens should be > 0, got {output_tokens}"
    );
}

/// The `input_tokens` from a streaming `message_start` should match the
/// count from the `POST /v1/messages/count_tokens` endpoint for the same body.
#[tokio::test]
async fn streaming_input_token_count_consistent_with_count_tokens_endpoint() {
    let (copilot_addr, _h1) = spawn_mock_copilot_text_response().await;
    let (github_addr, _h2) = spawn_mock_github().await;

    let state =
        super::test_helpers::create_test_state(format!("http://{copilot_addr}/chat/completions"), github_addr)
            .await;

    let request_body = streaming_request_body();

    // 1. Get count from /v1/messages/count_tokens
    let count_response = build_router(state.clone())
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/messages/count_tokens")
                .header("Content-Type", "application/json")
                .body(Body::from(serde_json::to_vec(&request_body).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(count_response.status(), StatusCode::OK);

    let count_bytes = axum::body::to_bytes(count_response.into_body(), usize::MAX)
        .await
        .unwrap();
    let count_json: serde_json::Value = serde_json::from_slice(&count_bytes).unwrap();
    let count_tokens_value = count_json["input_tokens"]
        .as_u64()
        .expect("count_tokens should return input_tokens");

    // 2. Get count from streaming message_start
    let app = build_router(state);
    let events = send_streaming_request(app, &request_body).await;

    let msg_start = events
        .iter()
        .find(|(t, _)| t == "message_start")
        .expect("should have message_start event");
    let streaming_input_tokens = msg_start.1["message"]["usage"]["input_tokens"]
        .as_u64()
        .expect("input_tokens should be a number");

    // 3. Both should agree
    assert_eq!(
        count_tokens_value, streaming_input_tokens,
        "count_tokens endpoint ({count_tokens_value}) should match streaming input_tokens ({streaming_input_tokens})"
    );
}

/// When the Copilot API returns upstream `usage` in a chunk, those values
/// should override the tiktoken estimates in the emitted events.
/// This test includes tools in the request to trigger the native tools
/// streaming path (which uses `StreamingState` with upstream usage capture).
#[tokio::test]
async fn upstream_usage_in_chunk_overrides_tiktoken() {
    let (copilot_addr, _h1) = spawn_mock_copilot_with_usage().await;
    let (github_addr, _h2) = spawn_mock_github().await;

    let state =
        super::test_helpers::create_test_state(format!("http://{copilot_addr}/chat/completions"), github_addr)
            .await;
    let app = build_router(state);

    // Include tools so the request goes through the native tools streaming
    // path, which uses StreamingState and captures upstream usage.
    let body = json!({
        "model": "claude-sonnet-4-20250514",
        "max_tokens": 8192,
        "messages": [{"role": "user", "content": "Hello, how are you?"}],
        "stream": true,
        "tools": [{
            "name": "test_tool",
            "description": "A test tool",
            "input_schema": {
                "type": "object",
                "properties": {
                    "arg": {"type": "string"}
                }
            }
        }]
    });

    let events = send_streaming_request(app, &body).await;

    // message_delta should use the upstream completion_tokens (888)
    let msg_delta = events
        .iter()
        .find(|(t, _)| t == "message_delta")
        .expect("should have message_delta event");

    let output_tokens = msg_delta.1["usage"]["output_tokens"]
        .as_u64()
        .expect("output_tokens should be a number");
    assert_eq!(
        output_tokens, 888,
        "output_tokens should be overridden by upstream completion_tokens, got {output_tokens}"
    );
}
