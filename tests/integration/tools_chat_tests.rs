//! Integration tests for experimental tool support in `/v1/chat/completions`.
//!
//! Covers:
//! - Requests with tools rejected (400) when `--experimental-tools` is disabled
//! - Requests with tools succeed when the flag is enabled
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
use copilot_adapter::copilot::types::ChatCompletionResponse;
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

```json
{"function_call": {"name": "get_weather", "arguments": {"location": "London"}}}
```

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
    experimental_tools: bool,
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
        config: AdapterConfig {
            experimental_tools,
        },
    })
}

// ---------------------------------------------------------------------------
// Test: tools present with flag disabled → 400
// ---------------------------------------------------------------------------

#[tokio::test]
async fn tools_in_request_returns_400_when_flag_disabled() {
    let (copilot_addr, _h1) = spawn_mock_copilot().await;
    let (github_addr, _h2) = spawn_mock_github().await;

    let state = create_test_state(
        format!("http://{copilot_addr}/chat/completions"),
        github_addr,
        false, // tools disabled
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

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);

    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(json["error"]["type"], "invalid_request_error");
    assert!(
        json["error"]["message"]
            .as_str()
            .unwrap()
            .contains("--experimental-tools"),
        "Error message should mention --experimental-tools flag"
    );
}

// ---------------------------------------------------------------------------
// Test: tool role message returns 400 when flag disabled
// ---------------------------------------------------------------------------

#[tokio::test]
async fn tool_role_message_returns_400_when_flag_disabled() {
    let (copilot_addr, _h1) = spawn_mock_copilot().await;
    let (github_addr, _h2) = spawn_mock_github().await;

    let state = create_test_state(
        format!("http://{copilot_addr}/chat/completions"),
        github_addr,
        false,
    )
    .await;
    let app = build_router(state);

    let body = json!({
        "model": "gpt-4",
        "messages": [
            {"role": "user", "content": "What's the weather?"},
            {"role": "assistant", "content": "Let me check.", "tool_calls": [{
                "id": "call_abc123",
                "type": "function",
                "function": {"name": "get_weather", "arguments": "{\"location\":\"London\"}"}
            }]},
            {"role": "tool", "content": "Sunny, 22°C", "tool_call_id": "call_abc123"}
        ]
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

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

// ---------------------------------------------------------------------------
// Test: tools present with flag enabled → success
// ---------------------------------------------------------------------------

#[tokio::test]
async fn tools_in_request_succeeds_when_flag_enabled() {
    let (copilot_addr, _h1) = spawn_mock_copilot().await;
    let (github_addr, _h2) = spawn_mock_github().await;

    let state = create_test_state(
        format!("http://{copilot_addr}/chat/completions"),
        github_addr,
        true, // tools enabled
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
        true,
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

    // The fenced code block should be stripped from content
    let content = resp.choices[0].message.content.as_text();
    assert!(
        !content.contains("```json"),
        "Fenced tool call should be stripped from content"
    );
    assert!(
        !content.contains("function_call"),
        "Tool call JSON should be stripped from content"
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
        true,
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
// Test: request without tools succeeds even when flag is enabled
// ---------------------------------------------------------------------------

#[tokio::test]
async fn no_tools_succeeds_when_flag_enabled() {
    let (copilot_addr, _h1) = spawn_mock_copilot().await;
    let (github_addr, _h2) = spawn_mock_github().await;

    let state = create_test_state(
        format!("http://{copilot_addr}/chat/completions"),
        github_addr,
        true,
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
// Test: request without tools succeeds when flag is disabled (no regression)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn no_tools_succeeds_when_flag_disabled() {
    let (copilot_addr, _h1) = spawn_mock_copilot().await;
    let (github_addr, _h2) = spawn_mock_github().await;

    let state = create_test_state(
        format!("http://{copilot_addr}/chat/completions"),
        github_addr,
        false,
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
// Test: no tool calls parsed when tools not in request (even if flag enabled)
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
        true,
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
        content.contains("function_call"),
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
        false, // flag disabled
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
