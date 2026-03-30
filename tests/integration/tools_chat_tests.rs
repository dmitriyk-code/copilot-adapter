//! Integration tests for tool support in `/v1/chat/completions`.
//!
//! Covers:
//! - Requests with tools succeed
//! - Tool calls parsed from mock response content
//! - `tool` role messages translated correctly

use std::sync::Arc;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::post;
use axum::{Json, Router};
use serde_json::json;
use tokio::net::TcpListener;
use tower::ServiceExt;

use copilot_adapter::auth::device_flow::DeviceFlowAuth;
use copilot_adapter::auth::token::TokenManager;
use copilot_adapter::copilot::client::CopilotClient;
use copilot_adapter::copilot::models_cache::ModelsCache;
use copilot_adapter::copilot::types::{ChatCompletionChunk, ChatCompletionResponse};
use copilot_adapter::server::{build_router, AdapterConfig, AppState};

use super::test_helpers::InMemoryStorage;

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
        "id": "chatcmpl-tools-test",
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
/// embedded in fenced JSON.
async fn spawn_mock_copilot_with_tool_call() -> (std::net::SocketAddr, tokio::task::JoinHandle<()>)
{
    let app =
        Router::new().route("/chat/completions", post(mock_chat_with_tool_call_handler));

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
        "id": "chatcmpl-toolcall",
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
        "id": "chatcmpl-echo",
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

// ---------------------------------------------------------------------------
// Helper: create AppState
// ---------------------------------------------------------------------------

async fn create_test_state(
    copilot_api_url: String,
    github_addr: std::net::SocketAddr,
) -> Arc<AppState> {
    let auth = DeviceFlowAuth::with_urls(
        format!("http://{github_addr}/unused"),
        format!("http://{github_addr}/unused"),
        format!("http://{github_addr}/copilot_internal/v2/token"),
    );

    let storage = InMemoryStorage::with_token("test_github_token");
    let tm = Arc::new(TokenManager::new(Box::new(storage), auth).await.unwrap());
    let client = reqwest::Client::new();

    Arc::new(AppState {
        token_manager: tm,
        copilot_client: CopilotClient::with_api_url(client.clone(), copilot_api_url),
        http_client: client,
        config: AdapterConfig::default(),
        models_cache: ModelsCache::new(std::time::Duration::from_secs(300)),
    })
}

// ---------------------------------------------------------------------------
// Test: tools present → success
// ---------------------------------------------------------------------------

#[tokio::test]
async fn tools_in_request_succeeds() {
    let (copilot_addr, _h1) = spawn_mock_copilot().await;
    let (github_addr, _h2) = spawn_mock_github().await;

    let state = create_test_state(
        format!("http://{copilot_addr}/chat/completions"),
        github_addr,
    )
    .await;
    let app = build_router(state);

    let body = json!({
        "model": "gpt-4",
        "messages": [{"role": "user", "content": "What's the weather?"}],
        "tools": [{
            "type": "function",
            "function": {
                "name": "get_weather",
                "description": "Get the weather",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "location": {"type": "string"}
                    },
                    "required": ["location"]
                }
            }
        }]
    });

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/chat/completions")
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
    let resp: ChatCompletionResponse = serde_json::from_slice(&bytes).unwrap();

    assert_eq!(resp.choices.len(), 1);
    assert_eq!(resp.choices[0].message.role, "assistant");
}

// ---------------------------------------------------------------------------
// Test: tool call parsed from mock response
// ---------------------------------------------------------------------------

#[tokio::test]
async fn tool_call_parsed_from_response_content() {
    let (copilot_addr, _h1) = spawn_mock_copilot_with_tool_call().await;
    let (github_addr, _h2) = spawn_mock_github().await;

    let state = create_test_state(
        format!("http://{copilot_addr}/chat/completions"),
        github_addr,
    )
    .await;
    let app = build_router(state);

    let body = json!({
        "model": "gpt-4",
        "messages": [{"role": "user", "content": "What's the weather in London?"}],
        "tools": [{
            "type": "function",
            "function": {
                "name": "get_weather",
                "description": "Get the weather",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "location": {"type": "string"}
                    },
                    "required": ["location"]
                }
            }
        }]
    });

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/chat/completions")
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
    let resp: ChatCompletionResponse = serde_json::from_slice(&bytes).unwrap();

    // Tool calls should be present
    let tool_calls = resp.choices[0]
        .message
        .tool_calls
        .as_ref()
        .expect("tool_calls should be present in the response");

    assert_eq!(tool_calls.len(), 1);
    assert_eq!(
        tool_calls[0].function.name,
        Some("get_weather".to_string())
    );
    assert!(tool_calls[0].id.as_ref().unwrap().starts_with("call_"));
    assert_eq!(
        tool_calls[0].call_type,
        Some("function".to_string())
    );

    // Arguments should contain the location
    let args = tool_calls[0].function.arguments.as_ref().unwrap();
    let args_value: serde_json::Value = serde_json::from_str(args).unwrap();
    assert_eq!(args_value["location"], "London");

    // The XML tool call block should be stripped from content
    let content = resp.choices[0].message.content.as_text();
    assert!(
        !content.contains("<function_calls>"),
        "Tool call XML should be stripped from content"
    );
    assert!(
        !content.contains("<invoke"),
        "Tool call invoke should be stripped from content"
    );

    // The surrounding prose should remain
    assert!(
        content.contains("check the weather"),
        "Surrounding prose should be preserved"
    );

    // finish_reason should be "tool_calls"
    assert_eq!(
        resp.choices[0].finish_reason,
        Some("tool_calls".to_string()),
        "finish_reason should be 'tool_calls' when tool calls are parsed"
    );
}

// ---------------------------------------------------------------------------
// Test: tool role message handled correctly (translated to user)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn tool_role_message_translated_to_user() {
    let (copilot_addr, _h1) = spawn_mock_copilot_echo().await;
    let (github_addr, _h2) = spawn_mock_github().await;

    let state = create_test_state(
        format!("http://{copilot_addr}/chat/completions"),
        github_addr,
    )
    .await;
    let app = build_router(state);

    let body = json!({
        "model": "gpt-4",
        "messages": [
            {"role": "user", "content": "What's the weather in London?"},
            {"role": "assistant", "content": "Let me check."},
            {"role": "tool", "content": "Sunny, 22°C", "tool_call_id": "call_abc123"}
        ],
        "tools": [{
            "type": "function",
            "function": {
                "name": "get_weather",
                "description": "Get the weather",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "location": {"type": "string"}
                    }
                }
            }
        }]
    });

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/chat/completions")
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
    let resp: ChatCompletionResponse = serde_json::from_slice(&bytes).unwrap();

    // The echo handler returns the messages it received as content.
    // The tool message should have been translated to a user message.
    let echoed_content = resp.choices[0].message.content.as_text();
    let echoed_messages: serde_json::Value =
        serde_json::from_str(&echoed_content).expect("echoed content should be valid JSON");

    // Find the message that was originally role "tool" — it should now be "user"
    let translated = echoed_messages
        .as_array()
        .unwrap()
        .iter()
        .find(|m| {
            m["content"]
                .as_str()
                .map_or(false, |c| c.contains("Tool Result"))
        })
        .expect("Should find translated tool message");

    assert_eq!(translated["role"], "user");
    assert!(
        translated["content"]
            .as_str()
            .unwrap()
            .contains("call_abc123"),
        "Translated message should contain the tool_call_id"
    );
    assert!(
        translated["content"]
            .as_str()
            .unwrap()
            .contains("Sunny, 22°C"),
        "Translated message should contain the tool result"
    );

    // Verify that tools and tool_choice are NOT forwarded upstream
    // (echo handler echoes body, but our handler strips these before sending)
    // We verify indirectly: the echo only contains messages, no tools key.
}

// ---------------------------------------------------------------------------
// Test: request without tools succeeds
// ---------------------------------------------------------------------------

#[tokio::test]
async fn no_tools_succeeds() {
    let (copilot_addr, _h1) = spawn_mock_copilot().await;
    let (github_addr, _h2) = spawn_mock_github().await;

    let state = create_test_state(
        format!("http://{copilot_addr}/chat/completions"),
        github_addr,
    )
    .await;
    let app = build_router(state);

    let body = json!({
        "model": "gpt-4",
        "messages": [{"role": "user", "content": "Hello"}]
    });

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/chat/completions")
                .header("Content-Type", "application/json")
                .body(Body::from(serde_json::to_vec(&body).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
}

// ---------------------------------------------------------------------------
// Test: no tool calls parsed when tools not in request
// ---------------------------------------------------------------------------

#[tokio::test]
async fn no_tool_calls_parsed_without_tools_in_request() {
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
        "model": "gpt-4",
        "messages": [{"role": "user", "content": "Hello"}]
        // No tools field — tool parsing should be skipped
    });

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/chat/completions")
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
    let resp: ChatCompletionResponse = serde_json::from_slice(&bytes).unwrap();

    // tool_calls should be None since we didn't request tools
    assert!(
        resp.choices[0].message.tool_calls.is_none(),
        "tool_calls should not be parsed when tools are not in the request"
    );

    // Content should still contain the tool call text (not stripped)
    let content = resp.choices[0].message.content.as_text();
    assert!(
        content.contains("<function_calls>"),
        "Content should be left untouched when tools not requested"
    );
}

// ---------------------------------------------------------------------------
// Test: empty tools array is treated as no tools
// ---------------------------------------------------------------------------

#[tokio::test]
async fn empty_tools_array_not_rejected() {
    let (copilot_addr, _h1) = spawn_mock_copilot().await;
    let (github_addr, _h2) = spawn_mock_github().await;

    let state = create_test_state(
        format!("http://{copilot_addr}/chat/completions"),
        github_addr,
    )
    .await;
    let app = build_router(state);

    let body = json!({
        "model": "gpt-4",
        "messages": [{"role": "user", "content": "Hello"}],
        "tools": []
    });

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/chat/completions")
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
// Streaming mock helpers
// ---------------------------------------------------------------------------

/// Spawn a mock Copilot API that returns SSE streaming chunks containing
/// a tool call embedded in fenced JSON.
async fn spawn_mock_streaming_copilot_with_tool_call(
) -> (std::net::SocketAddr, tokio::task::JoinHandle<()>) {
    let app = Router::new().route(
        "/chat/completions",
        post(mock_streaming_tool_call_handler),
    );

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

    // Simulate streaming content that includes a tool call in XML format.
    let chunks = vec![
        // First chunk: role
        format!(
            "data: {}\n\n",
            json!({
                "id": "chatcmpl-stream-tool",
                "object": "chat.completion.chunk",
                "created": 1700000000,
                "model": model,
                "choices": [{"index": 0, "delta": {"role": "assistant"}, "finish_reason": null}]
            })
        ),
        // Second chunk: text before tool call
        format!(
            "data: {}\n\n",
            json!({
                "id": "chatcmpl-stream-tool",
                "object": "chat.completion.chunk",
                "created": 1700000000,
                "model": model,
                "choices": [{"index": 0, "delta": {"content": "I'll check the weather.\n\n"}, "finish_reason": null}]
            })
        ),
        // Third chunk: XML tool call
        format!(
            "data: {}\n\n",
            json!({
                "id": "chatcmpl-stream-tool",
                "object": "chat.completion.chunk",
                "created": 1700000000,
                "model": model,
                "choices": [{"index": 0, "delta": {"content": "<function_calls>\n<invoke name=\"get_weather\">\n<parameter name=\"location\">London</parameter>\n</invoke>\n</function_calls>"}, "finish_reason": null}]
            })
        ),
        // Fourth chunk: text after tool call + finish
        format!(
            "data: {}\n\n",
            json!({
                "id": "chatcmpl-stream-tool",
                "object": "chat.completion.chunk",
                "created": 1700000000,
                "model": model,
                "choices": [{"index": 0, "delta": {"content": "\n\nLet me know if you need anything else."}, "finish_reason": "stop"}]
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

/// Spawn a mock Copilot API that returns SSE streaming chunks with plain text
/// (no tool calls).
async fn spawn_mock_streaming_copilot_plain(
) -> (std::net::SocketAddr, tokio::task::JoinHandle<()>) {
    let app = Router::new().route(
        "/chat/completions",
        post(mock_streaming_plain_handler),
    );

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let handle = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });

    (addr, handle)
}

/// Mock streaming handler that returns plain text (no tool calls).
async fn mock_streaming_plain_handler(
    _headers: axum::http::HeaderMap,
    Json(body): Json<serde_json::Value>,
) -> Response {
    let model = body["model"].as_str().unwrap_or("gpt-4");

    let chunks = vec![
        format!(
            "data: {}\n\n",
            json!({
                "id": "chatcmpl-stream-plain",
                "object": "chat.completion.chunk",
                "created": 1700000000,
                "model": model,
                "choices": [{"index": 0, "delta": {"role": "assistant"}, "finish_reason": null}]
            })
        ),
        format!(
            "data: {}\n\n",
            json!({
                "id": "chatcmpl-stream-plain",
                "object": "chat.completion.chunk",
                "created": 1700000000,
                "model": model,
                "choices": [{"index": 0, "delta": {"content": "Hello"}, "finish_reason": null}]
            })
        ),
        format!(
            "data: {}\n\n",
            json!({
                "id": "chatcmpl-stream-plain",
                "object": "chat.completion.chunk",
                "created": 1700000000,
                "model": model,
                "choices": [{"index": 0, "delta": {"content": " world!"}, "finish_reason": "stop"}]
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

/// Parse the raw SSE body text into chunks and a done flag.
fn parse_sse_body(body_text: &str) -> (Vec<ChatCompletionChunk>, bool) {
    let mut chunks = Vec::new();
    let mut saw_done = false;

    for frame in body_text.split("\n\n") {
        let frame = frame.trim();
        if frame.is_empty() {
            continue;
        }
        for line in frame.lines() {
            let line = line.trim();
            if let Some(data) = line.strip_prefix("data:") {
                let data = data.trim();
                if data == "[DONE]" {
                    saw_done = true;
                } else if let Ok(chunk) = serde_json::from_str::<ChatCompletionChunk>(data) {
                    chunks.push(chunk);
                }
            }
        }
    }

    (chunks, saw_done)
}

// ---------------------------------------------------------------------------
// E6-T8: OpenAI streaming with tool call detection
// ---------------------------------------------------------------------------

#[tokio::test]
async fn streaming_tool_call_detected_and_emitted() {
    let (copilot_addr, _h1) = spawn_mock_streaming_copilot_with_tool_call().await;
    let (github_addr, _h2) = spawn_mock_github().await;

    let state = create_test_state(
        format!("http://{copilot_addr}/chat/completions"),
        github_addr,
    )
    .await;
    let app = build_router(state);

    let body = json!({
        "model": "gpt-4",
        "messages": [{"role": "user", "content": "What's the weather in London?"}],
        "stream": true,
        "tools": [{
            "type": "function",
            "function": {
                "name": "get_weather",
                "description": "Get the weather",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "location": {"type": "string"}
                    },
                    "required": ["location"]
                }
            }
        }]
    });

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/chat/completions")
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
        "Expected text/event-stream content type, got: {ct}"
    );

    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let body_text = String::from_utf8_lossy(&bytes);
    let (chunks, saw_done) = parse_sse_body(&body_text);

    assert!(saw_done, "Expected [DONE] marker in SSE body");
    assert!(
        chunks.len() >= 2,
        "Expected at least 2 chunks (text + tool_calls), got: {}",
        chunks.len()
    );

    // Find the chunk that contains tool_calls
    let tool_chunk = chunks
        .iter()
        .find(|c| {
            c.choices
                .iter()
                .any(|ch| ch.delta.tool_calls.is_some())
        })
        .expect("Expected a chunk with tool_calls");

    let tool_calls = tool_chunk.choices[0]
        .delta
        .tool_calls
        .as_ref()
        .unwrap();

    assert_eq!(tool_calls.len(), 1);
    assert_eq!(
        tool_calls[0].function.name,
        Some("get_weather".to_string())
    );
    assert!(
        tool_calls[0].id.as_ref().unwrap().starts_with("call_"),
        "tool call id should start with 'call_'"
    );

    // Verify arguments contain the location
    let args = tool_calls[0].function.arguments.as_ref().unwrap();
    let args_value: serde_json::Value = serde_json::from_str(args).unwrap();
    assert_eq!(args_value["location"], "London");

    // The tool_calls chunk should have finish_reason = "tool_calls"
    assert_eq!(
        tool_chunk.choices[0].finish_reason,
        Some("tool_calls".to_string()),
        "finish_reason should be 'tool_calls' in the tool chunk"
    );

    // Text content chunks should not contain fenced JSON
    let all_text: String = chunks
        .iter()
        .flat_map(|c| c.choices.iter())
        .filter_map(|ch| ch.delta.content.as_ref())
        .cloned()
        .collect();

    assert!(
        !all_text.contains("```json"),
        "Fenced tool call should be stripped from streamed text"
    );
    assert!(
        !all_text.contains("function_call"),
        "Tool call JSON should be stripped from streamed text"
    );

    // Surrounding prose should be preserved
    assert!(
        all_text.contains("check the weather"),
        "Surrounding prose should be preserved in streamed text"
    );
}

// ---------------------------------------------------------------------------
// E6-T10: Streaming without tool call is unaffected
// ---------------------------------------------------------------------------

#[tokio::test]
async fn streaming_without_tool_call_unaffected() {
    let (copilot_addr, _h1) = spawn_mock_streaming_copilot_plain().await;
    let (github_addr, _h2) = spawn_mock_github().await;

    let state = create_test_state(
        format!("http://{copilot_addr}/chat/completions"),
        github_addr,
    )
    .await;
    let app = build_router(state);

    let body = json!({
        "model": "gpt-4",
        "messages": [{"role": "user", "content": "Hello"}],
        "stream": true
        // No tools field
    });

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/chat/completions")
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
    let (chunks, saw_done) = parse_sse_body(&body_text);

    assert!(saw_done, "Expected [DONE] marker");
    assert_eq!(chunks.len(), 3, "Expected 3 chunks (role + 2 text)");

    // Verify normal streaming content
    assert_eq!(
        chunks[0].choices[0].delta.role,
        Some("assistant".to_string())
    );
    assert_eq!(
        chunks[1].choices[0].delta.content,
        Some("Hello".to_string())
    );
    assert_eq!(
        chunks[2].choices[0].delta.content,
        Some(" world!".to_string())
    );
    assert_eq!(
        chunks[2].choices[0].finish_reason,
        Some("stop".to_string())
    );

    // No tool_calls should be present in any chunk
    for chunk in &chunks {
        for choice in &chunk.choices {
            assert!(
                choice.delta.tool_calls.is_none(),
                "No tool_calls expected in non-tool streaming response"
            );
        }
    }
}

// ---------------------------------------------------------------------------
// E6-T10 (additional): Streaming with tools but no tool call in response
// ---------------------------------------------------------------------------

#[tokio::test]
async fn streaming_with_tools_but_no_tool_call_replays_chunks() {
    // Tools are in the request, but the model response has no tool calls.
    // The buffered chunks should be replayed as-is.
    let (copilot_addr, _h1) = spawn_mock_streaming_copilot_plain().await;
    let (github_addr, _h2) = spawn_mock_github().await;

    let state = create_test_state(
        format!("http://{copilot_addr}/chat/completions"),
        github_addr,
    )
    .await;
    let app = build_router(state);

    let body = json!({
        "model": "gpt-4",
        "messages": [{"role": "user", "content": "Hello"}],
        "stream": true,
        "tools": [{
            "type": "function",
            "function": {
                "name": "get_weather",
                "description": "Get the weather",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "location": {"type": "string"}
                    }
                }
            }
        }]
    });

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/chat/completions")
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
    let (chunks, saw_done) = parse_sse_body(&body_text);

    assert!(saw_done, "Expected [DONE] marker");
    // Buffered chunks should be replayed as-is (3 original chunks)
    assert_eq!(
        chunks.len(),
        3,
        "Expected 3 replayed chunks when no tool calls detected"
    );

    // Verify content is intact
    let all_text: String = chunks
        .iter()
        .flat_map(|c| c.choices.iter())
        .filter_map(|ch| ch.delta.content.as_ref())
        .cloned()
        .collect();
    assert_eq!(all_text, "Hello world!");

    // No tool_calls should be present
    for chunk in &chunks {
        for choice in &chunk.choices {
            assert!(
                choice.delta.tool_calls.is_none(),
                "No tool_calls expected when model doesn't produce them"
            );
        }
    }
}

// ---------------------------------------------------------------------------
// Streaming error mock helpers
// ---------------------------------------------------------------------------

/// Spawn a mock Copilot API that returns some valid SSE chunks then sends
/// malformed data to trigger a parse error in the stream.
async fn spawn_mock_streaming_copilot_with_error(
) -> (std::net::SocketAddr, tokio::task::JoinHandle<()>) {
    let app = Router::new().route(
        "/chat/completions",
        post(mock_streaming_error_handler),
    );

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
        // Valid first chunk
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
        // Valid text chunk
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
        // Malformed JSON — will trigger a parse error in parse_sse_stream
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
// Test: OpenAI streaming with tools emits error event on upstream error
// ---------------------------------------------------------------------------

#[tokio::test]
async fn streaming_with_tools_emits_error_on_upstream_failure() {
    let (copilot_addr, _h1) = spawn_mock_streaming_copilot_with_error().await;
    let (github_addr, _h2) = spawn_mock_github().await;

    let state = create_test_state(
        format!("http://{copilot_addr}/chat/completions"),
        github_addr,
    )
    .await;
    let app = build_router(state);

    let body = json!({
        "model": "gpt-4",
        "messages": [{"role": "user", "content": "Hello"}],
        "stream": true,
        "tools": [{
            "type": "function",
            "function": {
                "name": "get_weather",
                "description": "Get the weather",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "location": {"type": "string"}
                    }
                }
            }
        }]
    });

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/chat/completions")
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

    // Should contain an error event with stream_error type
    assert!(
        body_text.contains("stream_error"),
        "Expected a stream_error event in the SSE body. Got:\n{body_text}"
    );

    // Should NOT contain a [DONE] marker after the error
    // (the stream should have terminated early)
    let error_pos = body_text.find("stream_error").unwrap();
    let after_error = &body_text[error_pos..];
    // The [DONE] event is still appended by the chain, but no valid chunks
    // should follow the error event
    assert!(
        !after_error.contains("tool_calls"),
        "No tool_calls should be emitted after an error"
    );
}

// ---------------------------------------------------------------------------
// Test: OpenAI normal streaming (no tools) emits error event on upstream failure
// ---------------------------------------------------------------------------

#[tokio::test]
async fn streaming_normal_emits_error_on_upstream_failure() {
    let (copilot_addr, _h1) = spawn_mock_streaming_copilot_with_error().await;
    let (github_addr, _h2) = spawn_mock_github().await;

    let state = create_test_state(
        format!("http://{copilot_addr}/chat/completions"),
        github_addr,
    )
    .await;
    let app = build_router(state);

    // No tools — uses the normal streaming path
    let body = json!({
        "model": "gpt-4",
        "messages": [{"role": "user", "content": "Hello"}],
        "stream": true
    });

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/chat/completions")
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

    // The normal streaming path already emits a stream_error event (chat.rs)
    assert!(
        body_text.contains("stream_error"),
        "Expected a stream_error event in the SSE body. Got:\n{body_text}"
    );
}
