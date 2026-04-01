//! Integration tests for tool support in `/v1/messages` (Anthropic format).
//!
//! Covers:
//! - Requests with tools succeed
//! - Tool calls returned as `tool_use` content blocks in response
//! - `tool_result` content blocks translated and forwarded correctly
//! - `stop_reason` is `"tool_use"` when tool calls are present

use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::post;
use axum::{Json, Router};
use serde_json::json;
use tokio::net::TcpListener;
use tower::ServiceExt;

use copilot_adapter::anthropic::types::AnthropicResponse;
use copilot_adapter::server::build_router;

use super::test_helpers::create_test_state;

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

/// Spawn a mock Copilot API that returns a normal text response.
async fn spawn_mock_copilot() -> (std::net::SocketAddr, tokio::task::JoinHandle<()>) {
    let app = Router::new().route("/chat/completions", post(mock_chat_handler));

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let handle = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });

    (addr, handle)
}

/// Mock handler: returns a plain text response.
async fn mock_chat_handler(
    _headers: axum::http::HeaderMap,
    Json(body): Json<serde_json::Value>,
) -> Response {
    let model = body["model"].as_str().unwrap_or("gpt-4");

    Json(json!({
        "id": "chatcmpl-tools-msg-test",
        "object": "chat.completion",
        "created": 1700000000,
        "model": model,
        "choices": [{
            "index": 0,
            "message": {
                "role": "assistant",
                "content": "Hello from mock Copilot!"
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
}

/// Spawn a mock Copilot API that returns a response containing a tool call
/// embedded in XML format.
async fn spawn_mock_copilot_with_tool_call() -> (std::net::SocketAddr, tokio::task::JoinHandle<()>)
{
    let app = Router::new().route("/chat/completions", post(mock_chat_with_tool_call_handler));

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let handle = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });

    (addr, handle)
}

/// Mock handler that returns content with an embedded tool call.
async fn mock_chat_with_tool_call_handler(
    _headers: axum::http::HeaderMap,
    Json(body): Json<serde_json::Value>,
) -> Response {
    let model = body["model"].as_str().unwrap_or("gpt-4");

    let content = r#"I'll check the weather for you.

<function_calls>
<invoke name="get_weather">
<parameter name="location">London</parameter>
</invoke>
</function_calls>

Let me know if you need anything else."#;

    Json(json!({
        "id": "chatcmpl-toolcall-msg",
        "object": "chat.completion",
        "created": 1700000000,
        "model": model,
        "choices": [{
            "index": 0,
            "message": {
                "role": "assistant",
                "content": content
            },
            "finish_reason": "stop"
        }],
        "usage": {
            "prompt_tokens": 20,
            "completion_tokens": 30,
            "total_tokens": 50
        }
    }))
    .into_response()
}

/// Spawn a mock Copilot API that echoes back the received messages for inspection.
async fn spawn_mock_copilot_echo() -> (std::net::SocketAddr, tokio::task::JoinHandle<()>) {
    let app = Router::new().route("/chat/completions", post(mock_chat_echo_handler));

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let handle = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });

    (addr, handle)
}

/// Mock handler that echoes the messages it received as the response content.
async fn mock_chat_echo_handler(
    _headers: axum::http::HeaderMap,
    Json(body): Json<serde_json::Value>,
) -> Response {
    let model = body["model"].as_str().unwrap_or("gpt-4");
    let messages = body["messages"].clone();
    let echo_content = serde_json::to_string_pretty(&messages).unwrap_or_default();

    Json(json!({
        "id": "chatcmpl-echo-msg",
        "object": "chat.completion",
        "created": 1700000000,
        "model": model,
        "choices": [{
            "index": 0,
            "message": {
                "role": "assistant",
                "content": echo_content
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
}

/// Helper to build a standard Anthropic tool definition.
fn sample_anthropic_tool() -> serde_json::Value {
    json!({
        "name": "get_weather",
        "description": "Get the current weather in a given location",
        "input_schema": {
            "type": "object",
            "properties": {
                "location": {
                    "type": "string",
                    "description": "The city name"
                }
            },
            "required": ["location"]
        }
    })
}

// ---------------------------------------------------------------------------
// E5-T10: Anthropic request with tools succeeds
// ---------------------------------------------------------------------------

#[tokio::test]
async fn anthropic_tools_succeeds() {
    let (copilot_addr, _h1) = spawn_mock_copilot().await;
    let (github_addr, _h2) = spawn_mock_github().await;

    let state = create_test_state(
        format!("http://{copilot_addr}/chat/completions"),
        github_addr,
    )
    .await;
    let app = build_router(state);

    let body = json!({
        "model": "claude-sonnet-4-20250514",
        "max_tokens": 1024,
        "messages": [{"role": "user", "content": "What's the weather?"}],
        "tools": [sample_anthropic_tool()]
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
    let resp: AnthropicResponse = serde_json::from_slice(&bytes).unwrap();

    assert_eq!(resp.response_type, "message");
    assert_eq!(resp.role, "assistant");
    assert!(!resp.content.is_empty());
}

// ---------------------------------------------------------------------------
// E5-T11: tool_use block in response
// ---------------------------------------------------------------------------

#[tokio::test]
async fn anthropic_tool_use_block_in_response() {
    let (copilot_addr, _h1) = spawn_mock_copilot_with_tool_call().await;
    let (github_addr, _h2) = spawn_mock_github().await;

    let state = create_test_state(
        format!("http://{copilot_addr}/chat/completions"),
        github_addr,
    )
    .await;
    let app = build_router(state);

    let body = json!({
        "model": "claude-sonnet-4-20250514",
        "max_tokens": 1024,
        "messages": [{"role": "user", "content": "What's the weather in London?"}],
        "tools": [sample_anthropic_tool()]
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
    let resp: AnthropicResponse = serde_json::from_slice(&bytes).unwrap();

    // Serialize to JSON Value for flexible assertions
    let resp_json = serde_json::to_value(&resp).unwrap();
    let content = resp_json["content"].as_array().unwrap();

    // Should have at least one text block (remaining prose) and one tool_use block
    let text_blocks: Vec<_> = content.iter().filter(|b| b["type"] == "text").collect();
    let tool_use_blocks: Vec<_> = content.iter().filter(|b| b["type"] == "tool_use").collect();

    assert!(
        !tool_use_blocks.is_empty(),
        "Response should contain at least one tool_use block"
    );

    // Verify tool_use block structure
    let tool_use = &tool_use_blocks[0];
    assert_eq!(tool_use["name"], "get_weather");
    assert!(
        tool_use["id"].as_str().unwrap().starts_with("call_"),
        "tool_use id should start with 'call_'"
    );
    assert_eq!(tool_use["input"]["location"], "London");

    // The surrounding prose should be in a text block
    if !text_blocks.is_empty() {
        let text_content = text_blocks[0]["text"].as_str().unwrap();
        assert!(
            text_content.contains("check the weather"),
            "Text block should contain surrounding prose"
        );
        assert!(
            !text_content.contains("<function_calls>"),
            "Tool call XML should be stripped from text"
        );
    }

    // stop_reason should be "tool_use" when tool calls are present
    assert_eq!(
        resp.stop_reason,
        Some("tool_use".to_string()),
        "stop_reason should be 'tool_use' when tool calls are present"
    );
}

// ---------------------------------------------------------------------------
// E5-T12: tool_result in request handled correctly
// ---------------------------------------------------------------------------

#[tokio::test]
async fn anthropic_tool_result_in_request_handled() {
    let (copilot_addr, _h1) = spawn_mock_copilot_echo().await;
    let (github_addr, _h2) = spawn_mock_github().await;

    let state = create_test_state(
        format!("http://{copilot_addr}/chat/completions"),
        github_addr,
    )
    .await;
    let app = build_router(state);

    let body = json!({
        "model": "claude-sonnet-4-20250514",
        "max_tokens": 1024,
        "messages": [
            {"role": "user", "content": "What's the weather in London?"},
            {"role": "assistant", "content": [
                {"type": "text", "text": "Let me check the weather."},
                {"type": "tool_use", "id": "call_abc123", "name": "get_weather", "input": {"location": "London"}}
            ]},
            {"role": "user", "content": [
                {"type": "tool_result", "tool_use_id": "call_abc123", "content": "Sunny, 22°C"}
            ]}
        ],
        "tools": [sample_anthropic_tool()]
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
    let resp: AnthropicResponse = serde_json::from_slice(&bytes).unwrap();

    // The echo handler returns the messages it received as content.
    // The tool_result should have been translated via tool-role messages
    // and then into user messages by the injector.
    let echoed_content = resp.content[0].text_content();
    let echoed_messages: serde_json::Value =
        serde_json::from_str(echoed_content).expect("echoed content should be valid JSON");

    // Find the message that was originally a tool_result — it should have been
    // translated from role "tool" to role "user" with tool result content.
    let translated = echoed_messages
        .as_array()
        .unwrap()
        .iter()
        .find(|m| {
            m["content"]
                .as_str()
                .map_or(false, |c| c.contains("Tool Result"))
        })
        .expect("Should find translated tool_result message");

    assert_eq!(translated["role"], "user");
    assert!(
        translated["content"]
            .as_str()
            .unwrap()
            .contains("call_abc123"),
        "Translated message should contain the tool_use_id"
    );
    assert!(
        translated["content"]
            .as_str()
            .unwrap()
            .contains("Sunny, 22°C"),
        "Translated message should contain the tool result content"
    );
}

// ---------------------------------------------------------------------------
// Additional: empty tools array not rejected
// ---------------------------------------------------------------------------

#[tokio::test]
async fn anthropic_empty_tools_array_not_rejected() {
    let (copilot_addr, _h1) = spawn_mock_copilot().await;
    let (github_addr, _h2) = spawn_mock_github().await;

    let state = create_test_state(
        format!("http://{copilot_addr}/chat/completions"),
        github_addr,
    )
    .await;
    let app = build_router(state);

    let body = json!({
        "model": "claude-sonnet-4-20250514",
        "max_tokens": 1024,
        "messages": [{"role": "user", "content": "Hello"}],
        "tools": []
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

    // Empty tools array should NOT be rejected
    assert_eq!(response.status(), StatusCode::OK);
}

// ---------------------------------------------------------------------------
// Additional: request without tools succeeds
// ---------------------------------------------------------------------------

#[tokio::test]
async fn anthropic_no_tools_succeeds() {
    let (copilot_addr, _h1) = spawn_mock_copilot().await;
    let (github_addr, _h2) = spawn_mock_github().await;

    let state = create_test_state(
        format!("http://{copilot_addr}/chat/completions"),
        github_addr,
    )
    .await;
    let app = build_router(state);

    let body = json!({
        "model": "claude-sonnet-4-20250514",
        "max_tokens": 1024,
        "messages": [{"role": "user", "content": "Hello"}]
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
}

// ---------------------------------------------------------------------------
// Additional: no tool calls parsed when tools not in request
// ---------------------------------------------------------------------------

#[tokio::test]
async fn anthropic_no_tool_calls_parsed_without_tools_in_request() {
    // This mock returns content that looks like a tool call,
    // but since no tools were sent in the request, we should NOT parse it.
    let (copilot_addr, _h1) = spawn_mock_copilot_with_tool_call().await;
    let (github_addr, _h2) = spawn_mock_github().await;

    let state = create_test_state(
        format!("http://{copilot_addr}/chat/completions"),
        github_addr,
    )
    .await;
    let app = build_router(state);

    let body = json!({
        "model": "claude-sonnet-4-20250514",
        "max_tokens": 1024,
        "messages": [{"role": "user", "content": "Hello"}]
        // No tools field — tool parsing should be skipped
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
    let resp: AnthropicResponse = serde_json::from_slice(&bytes).unwrap();

    // Should be a single text block (tool calls not parsed)
    let resp_json = serde_json::to_value(&resp).unwrap();
    let content = resp_json["content"].as_array().unwrap();

    assert_eq!(content.len(), 1, "Should have exactly one content block");
    assert_eq!(
        content[0]["type"], "text",
        "Content block should be text type"
    );

    // Content should still contain the raw tool call text (not stripped)
    let text = content[0]["text"].as_str().unwrap();
    assert!(
        text.contains("<function_calls>"),
        "Content should be left untouched when tools not requested"
    );

    // stop_reason should NOT be "tool_use"
    assert_ne!(
        resp.stop_reason,
        Some("tool_use".to_string()),
        "stop_reason should not be 'tool_use' when tools not requested"
    );
}

// ---------------------------------------------------------------------------
// Streaming mock helpers
// ---------------------------------------------------------------------------

/// Spawn a mock Copilot API that returns SSE streaming chunks containing
/// a tool call embedded in XML format.
async fn spawn_mock_streaming_copilot_with_tool_call(
) -> (std::net::SocketAddr, tokio::task::JoinHandle<()>) {
    let app = Router::new().route("/chat/completions", post(mock_streaming_tool_call_handler));

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let handle = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });

    (addr, handle)
}

/// Mock streaming handler that returns content with an embedded tool call
/// spread across SSE chunks.
async fn mock_streaming_tool_call_handler(
    _headers: axum::http::HeaderMap,
    Json(body): Json<serde_json::Value>,
) -> Response {
    let model = body["model"].as_str().unwrap_or("gpt-4");

    let chunks = vec![
        format!(
            "data: {}\n\n",
            json!({
                "id": "chatcmpl-stream-tool-msg",
                "object": "chat.completion.chunk",
                "created": 1700000000,
                "model": model,
                "choices": [{"index": 0, "delta": {"role": "assistant"}, "finish_reason": null}]
            })
        ),
        format!(
            "data: {}\n\n",
            json!({
                "id": "chatcmpl-stream-tool-msg",
                "object": "chat.completion.chunk",
                "created": 1700000000,
                "model": model,
                "choices": [{"index": 0, "delta": {"content": "I'll check the weather.\n\n"}, "finish_reason": null}]
            })
        ),
        format!(
            "data: {}\n\n",
            json!({
                "id": "chatcmpl-stream-tool-msg",
                "object": "chat.completion.chunk",
                "created": 1700000000,
                "model": model,
                "choices": [{"index": 0, "delta": {"content": "<function_calls>\n<invoke name=\"get_weather\">\n<parameter name=\"location\">London</parameter>\n</invoke>\n</function_calls>"}, "finish_reason": null}]
            })
        ),
        format!(
            "data: {}\n\n",
            json!({
                "id": "chatcmpl-stream-tool-msg",
                "object": "chat.completion.chunk",
                "created": 1700000000,
                "model": model,
                "choices": [{"index": 0, "delta": {"content": "\n\nLet me know."}, "finish_reason": "stop"}]
            })
        ),
        "data: [DONE]\n\n".to_string(),
    ];

    let sse_body: String = chunks.concat();

    axum::http::Response::builder()
        .status(200)
        .header("Content-Type", "text/event-stream")
        .body(Body::from(sse_body))
        .unwrap()
}

// ---------------------------------------------------------------------------
// E6-T9: Anthropic streaming with tool call detection
// ---------------------------------------------------------------------------

#[tokio::test]
async fn anthropic_streaming_tool_call_detected_and_emitted() {
    let (copilot_addr, _h1) = spawn_mock_streaming_copilot_with_tool_call().await;
    let (github_addr, _h2) = spawn_mock_github().await;

    let state = create_test_state(
        format!("http://{copilot_addr}/chat/completions"),
        github_addr,
    )
    .await;
    let app = build_router(state);

    let body = json!({
        "model": "claude-sonnet-4-20250514",
        "max_tokens": 1024,
        "messages": [{"role": "user", "content": "What's the weather in London?"}],
        "stream": true,
        "tools": [sample_anthropic_tool()]
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

    let ct = response
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    assert!(
        ct.contains("text/event-stream"),
        "Expected text/event-stream, got: {ct}"
    );

    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let body_text = String::from_utf8_lossy(&bytes);

    // Parse SSE events from the response
    let events = parse_anthropic_sse_events(&body_text);

    // Should have message_start
    let message_start = events
        .iter()
        .find(|e| e["type"] == "message_start")
        .expect("Expected message_start event");
    assert_eq!(message_start["message"]["role"], "assistant");

    // Should have at least one content_block_start with type "tool_use"
    let tool_use_starts: Vec<_> = events
        .iter()
        .filter(|e| e["type"] == "content_block_start" && e["content_block"]["type"] == "tool_use")
        .collect();
    assert!(
        !tool_use_starts.is_empty(),
        "Expected at least one tool_use content_block_start event"
    );

    let tool_block = &tool_use_starts[0]["content_block"];
    assert_eq!(tool_block["name"], "get_weather");
    assert!(
        tool_block["id"].as_str().unwrap().starts_with("call_"),
        "tool_use id should start with 'call_'"
    );

    // Should have an input_json_delta event with the tool arguments
    let input_deltas: Vec<_> = events
        .iter()
        .filter(|e| e["type"] == "content_block_delta" && e["delta"]["type"] == "input_json_delta")
        .collect();
    assert!(
        !input_deltas.is_empty(),
        "Expected at least one input_json_delta event"
    );

    // Collect and concatenate all partial_json slices — the Anthropic SSE spec
    // allows `partial_json` to be fragmented across multiple delta events.
    let combined_json: String = input_deltas
        .iter()
        .filter_map(|e| e["delta"]["partial_json"].as_str())
        .collect();
    let input: serde_json::Value = serde_json::from_str(&combined_json).unwrap();
    assert_eq!(input["location"], "London");

    // Text content should not contain the XML tool call markup
    let text_deltas: Vec<_> = events
        .iter()
        .filter(|e| e["type"] == "content_block_delta" && e["delta"]["type"] == "text_delta")
        .collect();

    let all_text: String = text_deltas
        .iter()
        .filter_map(|e| e["delta"]["text"].as_str())
        .collect();

    assert!(
        !all_text.contains("<function_calls>"),
        "XML tool call should be stripped from text deltas"
    );
    assert!(
        !all_text.contains("<invoke"),
        "Invoke XML should be stripped from text deltas"
    );
    assert!(
        all_text.contains("check the weather"),
        "Surrounding prose should be in text deltas"
    );

    // message_delta should have stop_reason = "tool_use"
    let message_delta = events
        .iter()
        .find(|e| e["type"] == "message_delta")
        .expect("Expected message_delta event");
    assert_eq!(
        message_delta["delta"]["stop_reason"], "tool_use",
        "stop_reason should be 'tool_use' when tool calls are present"
    );

    // Should have message_stop
    assert!(
        events.iter().any(|e| e["type"] == "message_stop"),
        "Expected message_stop event"
    );
}

/// Parse Anthropic SSE events from raw response body text.
fn parse_anthropic_sse_events(body_text: &str) -> Vec<serde_json::Value> {
    let mut events = Vec::new();

    for line in body_text.lines() {
        let line = line.trim();
        if let Some(data) = line.strip_prefix("data:") {
            let data = data.trim();
            if data.is_empty() || data == "[DONE]" {
                continue;
            }
            if let Ok(value) = serde_json::from_str::<serde_json::Value>(data) {
                events.push(value);
            }
        }
    }

    events
}

// ---------------------------------------------------------------------------
// Streaming error mock helpers
// ---------------------------------------------------------------------------

/// Spawn a mock Copilot API that returns some valid SSE chunks then sends
/// malformed data to trigger a parse error in the stream.
async fn spawn_mock_streaming_copilot_with_error(
) -> (std::net::SocketAddr, tokio::task::JoinHandle<()>) {
    let app = Router::new().route("/chat/completions", post(mock_streaming_error_handler));

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let handle = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });

    (addr, handle)
}

/// Mock streaming handler that returns a valid chunk then invalid JSON,
/// causing a parse error downstream.
async fn mock_streaming_error_handler(
    _headers: axum::http::HeaderMap,
    Json(body): Json<serde_json::Value>,
) -> Response {
    let model = body["model"].as_str().unwrap_or("gpt-4");

    let chunks = vec![
        format!(
            "data: {}\n\n",
            json!({
                "id": "chatcmpl-stream-err",
                "object": "chat.completion.chunk",
                "created": 1700000000,
                "model": model,
                "choices": [{"index": 0, "delta": {"role": "assistant"}, "finish_reason": null}]
            })
        ),
        format!(
            "data: {}\n\n",
            json!({
                "id": "chatcmpl-stream-err",
                "object": "chat.completion.chunk",
                "created": 1700000000,
                "model": model,
                "choices": [{"index": 0, "delta": {"content": "partial"}, "finish_reason": null}]
            })
        ),
        // Malformed JSON — triggers a parse error
        "data: {invalid json here}\n\n".to_string(),
    ];

    let sse_body: String = chunks.concat();

    axum::http::Response::builder()
        .status(200)
        .header("Content-Type", "text/event-stream")
        .body(Body::from(sse_body))
        .unwrap()
}

// ---------------------------------------------------------------------------
// Test: Anthropic streaming with tools emits error event on upstream error
// ---------------------------------------------------------------------------

#[tokio::test]
async fn anthropic_streaming_with_tools_emits_error_on_upstream_failure() {
    let (copilot_addr, _h1) = spawn_mock_streaming_copilot_with_error().await;
    let (github_addr, _h2) = spawn_mock_github().await;

    let state = create_test_state(
        format!("http://{copilot_addr}/chat/completions"),
        github_addr,
    )
    .await;
    let app = build_router(state);

    let body = json!({
        "model": "claude-sonnet-4-20250514",
        "max_tokens": 1024,
        "messages": [{"role": "user", "content": "Hello"}],
        "stream": true,
        "tools": [sample_anthropic_tool()]
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

    // Should contain an Anthropic-format error event
    assert!(
        body_text.contains("api_error"),
        "Expected an Anthropic-format error event with 'api_error' type. Got:\n{body_text}"
    );

    // The error event should be structured as Anthropic expects
    let events = parse_anthropic_sse_events(&body_text);
    let error_events: Vec<_> = events.iter().filter(|e| e["type"] == "error").collect();
    assert!(
        !error_events.is_empty(),
        "Expected at least one error event in the stream"
    );
    assert!(
        error_events[0]["error"]["type"] == "api_error",
        "Error event should have type 'api_error'"
    );

    // No tool_use blocks should be emitted after the error
    let tool_use_blocks: Vec<_> = events
        .iter()
        .filter(|e| e["type"] == "content_block_start" && e["content_block"]["type"] == "tool_use")
        .collect();
    assert!(
        tool_use_blocks.is_empty(),
        "No tool_use blocks should be emitted after an upstream error"
    );
}

// ---------------------------------------------------------------------------
// Test: Anthropic normal streaming (no tools) emits error event on upstream error
// ---------------------------------------------------------------------------

#[tokio::test]
async fn anthropic_streaming_normal_emits_error_on_upstream_failure() {
    let (copilot_addr, _h1) = spawn_mock_streaming_copilot_with_error().await;
    let (github_addr, _h2) = spawn_mock_github().await;

    let state = create_test_state(
        format!("http://{copilot_addr}/chat/completions"),
        github_addr,
    )
    .await;
    let app = build_router(state);

    // No tools — uses the normal streaming path in handle_streaming
    let body = json!({
        "model": "claude-sonnet-4-20250514",
        "max_tokens": 1024,
        "messages": [{"role": "user", "content": "Hello"}],
        "stream": true
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

    // Should contain an Anthropic-format error event
    assert!(
        body_text.contains("api_error"),
        "Expected an Anthropic-format error event. Got:\n{body_text}"
    );

    // The error event should be properly structured
    let events = parse_anthropic_sse_events(&body_text);
    let error_events: Vec<_> = events.iter().filter(|e| e["type"] == "error").collect();
    assert!(
        !error_events.is_empty(),
        "Expected at least one error event in the stream"
    );

    // No message_delta or message_stop should follow the error
    // (the stream should terminate after the error event)
    let error_idx = events.iter().position(|e| e["type"] == "error").unwrap();
    let after_error: Vec<_> = events[error_idx + 1..].to_vec();
    assert!(
        after_error.is_empty(),
        "No events should follow the error event, but got: {after_error:?}"
    );
}
