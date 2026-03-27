use std::sync::Arc;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::post;
use axum::{Json, Router};
use serde_json::json;
use tokio::net::TcpListener;
use tower::ServiceExt;

use copilot_adapter::anthropic::types::AnthropicResponse;
use copilot_adapter::auth::device_flow::DeviceFlowAuth;
use copilot_adapter::auth::token::TokenManager;
use copilot_adapter::copilot::client::CopilotClient;
use copilot_adapter::server::{build_router, AppState};

use super::test_helpers::InMemoryStorage;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Spawn a mock Copilot API that handles both streaming and non-streaming.
async fn spawn_mock_copilot_api() -> (std::net::SocketAddr, tokio::task::JoinHandle<()>) {
    let app = Router::new().route("/chat/completions", post(mock_chat_completions));
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let handle = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    (addr, handle)
}

/// Mock chat completions endpoint supporting both streaming and non-streaming.
async fn mock_chat_completions(
    axum::Json(body): axum::Json<serde_json::Value>,
) -> Response {
    let model = body["model"].as_str().unwrap_or("gpt-4");
    let stream = body["stream"].as_bool().unwrap_or(false);

    if stream {
        let chunks = vec![
            format!(
                "data: {}\n\n",
                json!({
                    "id": "chatcmpl-mock123",
                    "object": "chat.completion.chunk",
                    "created": 1700000000,
                    "model": model,
                    "choices": [{"index": 0, "delta": {"role": "assistant"}, "finish_reason": null}]
                })
            ),
            format!(
                "data: {}\n\n",
                json!({
                    "id": "chatcmpl-mock123",
                    "object": "chat.completion.chunk",
                    "created": 1700000000,
                    "model": model,
                    "choices": [{"index": 0, "delta": {"content": "Hello"}, "finish_reason": null}]
                })
            ),
            format!(
                "data: {}\n\n",
                json!({
                    "id": "chatcmpl-mock123",
                    "object": "chat.completion.chunk",
                    "created": 1700000000,
                    "model": model,
                    "choices": [{"index": 0, "delta": {"content": " world!"}, "finish_reason": "stop"}]
                })
            ),
            "data: [DONE]\n\n".to_string(),
        ];

        return axum::http::Response::builder()
            .status(200)
            .header("Content-Type", "text/event-stream")
            .body(Body::from(chunks.concat()))
            .unwrap()
            .into_response();
    }

    Json(json!({
        "id": "chatcmpl-mock123",
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

/// Create test AppState pointing at mock servers.
async fn create_test_state(
    copilot_api_url: String,
    github_api_addr: std::net::SocketAddr,
) -> Arc<AppState> {
    let auth = DeviceFlowAuth::with_urls(
        format!("http://{github_api_addr}/unused"),
        format!("http://{github_api_addr}/unused"),
        format!("http://{github_api_addr}/copilot_internal/v2/token"),
    );

    let storage = InMemoryStorage::with_token("test_github_token");
    let tm = Arc::new(TokenManager::new(Box::new(storage), auth).await.unwrap());
    let client = reqwest::Client::new();

    Arc::new(AppState {
        token_manager: tm,
        copilot_client: CopilotClient::with_api_url(client.clone(), copilot_api_url),
        http_client: client,
    })
}

/// Parse Anthropic SSE body into events. Returns (events, event_types).
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

// ---------------------------------------------------------------------------
// E8-T12: Non-streaming integration tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn messages_non_streaming_returns_anthropic_format() {
    let (copilot_addr, _h1) = spawn_mock_copilot_api().await;
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

    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let resp: AnthropicResponse = serde_json::from_slice(&bytes).unwrap();

    assert_eq!(resp.response_type, "message");
    assert_eq!(resp.role, "assistant");
    assert_eq!(resp.content.len(), 1);
    assert_eq!(resp.content[0].block_type, "text");
    assert_eq!(resp.content[0].text, "Hello from mock Copilot!");
    assert_eq!(resp.stop_reason, Some("end_turn".to_string()));
    assert_eq!(resp.usage.input_tokens, 10);
    assert_eq!(resp.usage.output_tokens, 5);
}

#[tokio::test]
async fn messages_non_streaming_with_system_prompt() {
    let (copilot_addr, _h1) = spawn_mock_copilot_api().await;
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
        "system": "You are a helpful assistant.",
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

    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let resp: AnthropicResponse = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(resp.response_type, "message");
    assert_eq!(resp.content[0].text, "Hello from mock Copilot!");
}

#[tokio::test]
async fn messages_non_streaming_with_content_blocks() {
    let (copilot_addr, _h1) = spawn_mock_copilot_api().await;
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
        "messages": [{
            "role": "user",
            "content": [
                {"type": "text", "text": "Hello "},
                {"type": "text", "text": "world!"}
            ]
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
    let resp: AnthropicResponse = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(resp.response_type, "message");
}

#[tokio::test]
async fn messages_empty_messages_returns_400() {
    let (copilot_addr, _h1) = spawn_mock_copilot_api().await;
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
        "messages": []
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

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn messages_response_has_correct_json_structure() {
    let (copilot_addr, _h1) = spawn_mock_copilot_api().await;
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

    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();

    // Verify Anthropic response structure
    assert!(json.get("id").is_some(), "response must have 'id'");
    assert_eq!(json["type"], "message");
    assert_eq!(json["role"], "assistant");
    assert!(json.get("content").is_some(), "response must have 'content'");
    assert!(json["content"].is_array());
    assert_eq!(json["content"][0]["type"], "text");
    assert!(json.get("usage").is_some(), "response must have 'usage'");
    assert!(
        json["usage"].get("input_tokens").is_some(),
        "usage must have 'input_tokens'"
    );
    assert!(
        json["usage"].get("output_tokens").is_some(),
        "usage must have 'output_tokens'"
    );
}

// ---------------------------------------------------------------------------
// E8-T13: Streaming integration tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn messages_streaming_returns_anthropic_sse_events() {
    let (copilot_addr, _h1) = spawn_mock_copilot_api().await;
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

    // Content-Type should indicate SSE
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

    let events = parse_anthropic_sse(&body_text);

    // Verify event types in order
    let event_types: Vec<&str> = events.iter().map(|(t, _)| t.as_str()).collect();
    assert!(
        event_types.contains(&"message_start"),
        "Should contain message_start, got: {event_types:?}"
    );
    assert!(
        event_types.contains(&"content_block_start"),
        "Should contain content_block_start, got: {event_types:?}"
    );
    assert!(
        event_types.contains(&"content_block_delta"),
        "Should contain content_block_delta, got: {event_types:?}"
    );
    assert!(
        event_types.contains(&"content_block_stop"),
        "Should contain content_block_stop, got: {event_types:?}"
    );
    assert!(
        event_types.contains(&"message_delta"),
        "Should contain message_delta, got: {event_types:?}"
    );
    assert!(
        event_types.contains(&"message_stop"),
        "Should contain message_stop, got: {event_types:?}"
    );
}

#[tokio::test]
async fn messages_streaming_message_start_has_correct_structure() {
    let (copilot_addr, _h1) = spawn_mock_copilot_api().await;
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

    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let body_text = String::from_utf8_lossy(&bytes);
    let events = parse_anthropic_sse(&body_text);

    // Find message_start event
    let (_, msg_start) = events
        .iter()
        .find(|(t, _)| t == "message_start")
        .expect("should have message_start event");

    assert_eq!(msg_start["type"], "message_start");
    assert!(msg_start.get("message").is_some());
    assert_eq!(msg_start["message"]["type"], "message");
    assert_eq!(msg_start["message"]["role"], "assistant");
}

#[tokio::test]
async fn messages_streaming_content_deltas_contain_text() {
    let (copilot_addr, _h1) = spawn_mock_copilot_api().await;
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

    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let body_text = String::from_utf8_lossy(&bytes);
    let events = parse_anthropic_sse(&body_text);

    // Collect all content_block_delta events
    let deltas: Vec<&serde_json::Value> = events
        .iter()
        .filter(|(t, _)| t == "content_block_delta")
        .map(|(_, v)| v)
        .collect();

    assert!(!deltas.is_empty(), "Should have at least one content_block_delta");

    // Each delta should have text_delta type
    for delta in &deltas {
        assert_eq!(delta["delta"]["type"], "text_delta");
        assert!(delta["delta"].get("text").is_some());
    }

    // Concatenate all delta text
    let full_text: String = deltas
        .iter()
        .map(|d| d["delta"]["text"].as_str().unwrap_or(""))
        .collect();
    assert_eq!(full_text, "Hello world!");
}

#[tokio::test]
async fn messages_streaming_message_delta_has_stop_reason() {
    let (copilot_addr, _h1) = spawn_mock_copilot_api().await;
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

    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let body_text = String::from_utf8_lossy(&bytes);
    let events = parse_anthropic_sse(&body_text);

    // Find message_delta event
    let (_, msg_delta) = events
        .iter()
        .find(|(t, _)| t == "message_delta")
        .expect("should have message_delta event");

    assert_eq!(msg_delta["type"], "message_delta");
    assert_eq!(msg_delta["delta"]["stop_reason"], "end_turn");
    assert!(msg_delta.get("usage").is_some());
    assert!(msg_delta["usage"].get("output_tokens").is_some());
}

#[tokio::test]
async fn messages_streaming_event_order_is_correct() {
    let (copilot_addr, _h1) = spawn_mock_copilot_api().await;
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

    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let body_text = String::from_utf8_lossy(&bytes);
    let events = parse_anthropic_sse(&body_text);
    let event_types: Vec<&str> = events.iter().map(|(t, _)| t.as_str()).collect();

    // Verify ordering: message_start must come first, message_stop must come last
    let msg_start_idx = event_types
        .iter()
        .position(|t| *t == "message_start")
        .expect("message_start");
    let content_start_idx = event_types
        .iter()
        .position(|t| *t == "content_block_start")
        .expect("content_block_start");
    let content_stop_idx = event_types
        .iter()
        .rposition(|t| *t == "content_block_stop")
        .expect("content_block_stop");
    let msg_delta_idx = event_types
        .iter()
        .position(|t| *t == "message_delta")
        .expect("message_delta");
    let msg_stop_idx = event_types
        .iter()
        .position(|t| *t == "message_stop")
        .expect("message_stop");

    assert!(msg_start_idx < content_start_idx);
    assert!(content_start_idx < content_stop_idx);
    assert!(content_stop_idx < msg_delta_idx);
    assert!(msg_delta_idx < msg_stop_idx);
}
