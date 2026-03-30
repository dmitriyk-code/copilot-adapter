use std::sync::Arc;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::post;
use axum::{Json, Router};
use serde_json::json;
use tokio::net::TcpListener;
use tokio::sync::Mutex;
use tower::ServiceExt;

use copilot_adapter::anthropic::types::AnthropicResponse;
use copilot_adapter::auth::device_flow::DeviceFlowAuth;
use copilot_adapter::auth::token::TokenManager;
use copilot_adapter::copilot::client::CopilotClient;
use copilot_adapter::copilot::models_cache::ModelsCache;
use copilot_adapter::server::{build_router, AdapterConfig, AppState};

use super::test_helpers::InMemoryStorage;

// ---------------------------------------------------------------------------
// Helpers — mock Copilot that captures the request body
// ---------------------------------------------------------------------------

/// Shared state for the capturing mock: stores the last received request body.
type CapturedBody = Arc<Mutex<Option<serde_json::Value>>>;

/// Spawn a mock Copilot API that captures the request body for later inspection.
async fn spawn_capturing_mock_copilot() -> (
    std::net::SocketAddr,
    CapturedBody,
    tokio::task::JoinHandle<()>,
) {
    let captured: CapturedBody = Arc::new(Mutex::new(None));
    let captured_clone = captured.clone();

    let app = Router::new().route(
        "/chat/completions",
        post(move |axum::Json(body): axum::Json<serde_json::Value>| {
            let cap = captured_clone.clone();
            async move {
                *cap.lock().await = Some(body.clone());
                let model = body["model"].as_str().unwrap_or("gpt-4");
                Json(json!({
                    "id": "chatcmpl-mock-multimodal",
                    "object": "chat.completion",
                    "created": 1700000000,
                    "model": model,
                    "choices": [{
                        "index": 0,
                        "message": {
                            "role": "assistant",
                            "content": "I can see the image."
                        },
                        "finish_reason": "stop"
                    }],
                    "usage": {
                        "prompt_tokens": 20,
                        "completion_tokens": 6,
                        "total_tokens": 26
                    }
                }))
                .into_response()
            }
        }),
    );

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let handle = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    (addr, captured, handle)
}

/// Spawn a mock GitHub token server.
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
        copilot_client: CopilotClient::with_api_url(client, copilot_api_url),
        config: AdapterConfig::default(),
        models_cache: ModelsCache::new(std::time::Duration::from_secs(300)),
    })
}

/// Send a POST /v1/messages request and return the response.
async fn send_messages_request(
    app: Router,
    body: serde_json::Value,
) -> Response {
    app.oneshot(
        Request::builder()
            .method("POST")
            .uri("/v1/messages")
            .header("Content-Type", "application/json")
            .body(Body::from(serde_json::to_vec(&body).unwrap()))
            .unwrap(),
    )
    .await
    .unwrap()
}

// ---------------------------------------------------------------------------
// E4-T1: Integration test with image block (base64)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn image_upload_base64() {
    let (copilot_addr, captured, _h1) = spawn_capturing_mock_copilot().await;
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
                {"type": "text", "text": "What is in this image?"},
                {
                    "type": "image",
                    "source": {
                        "type": "base64",
                        "media_type": "image/png",
                        "data": "iVBORw0KGgoAAAANSUhEUg=="
                    }
                }
            ]
        }]
    });

    let response = send_messages_request(app, body).await;

    // Should not return 422 — image blocks deserialize without errors
    assert_eq!(response.status(), StatusCode::OK);

    // Parse the response as valid Anthropic format
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let resp: AnthropicResponse = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(resp.response_type, "message");
    assert_eq!(resp.role, "assistant");
    assert_eq!(resp.content[0].text_content(), "I can see the image.");

    // Verify the mock Copilot received the correct OpenAI multimodal format
    let captured_body = captured.lock().await;
    let body = captured_body.as_ref().expect("mock should have captured a request");
    let messages = body["messages"].as_array().unwrap();
    let user_msg = &messages[0];
    assert_eq!(user_msg["role"], "user");

    // Content should be an array of content blocks (multimodal)
    let content = user_msg["content"].as_array().expect("content should be an array");
    assert_eq!(content.len(), 2);

    // First block: text
    assert_eq!(content[0]["type"], "text");
    assert_eq!(content[0]["text"], "What is in this image?");

    // Second block: image_url with data URI
    assert_eq!(content[1]["type"], "image_url");
    assert_eq!(
        content[1]["image_url"]["url"],
        "data:image/png;base64,iVBORw0KGgoAAAANSUhEUg=="
    );
}

// ---------------------------------------------------------------------------
// E4-T2: Integration test with image block (URL)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn image_upload_url() {
    let (copilot_addr, captured, _h1) = spawn_capturing_mock_copilot().await;
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
                {"type": "text", "text": "Describe this photo."},
                {
                    "type": "image",
                    "source": {
                        "type": "url",
                        "url": "https://example.com/photo.jpg"
                    }
                }
            ]
        }]
    });

    let response = send_messages_request(app, body).await;

    assert_eq!(response.status(), StatusCode::OK);

    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let resp: AnthropicResponse = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(resp.response_type, "message");
    assert_eq!(resp.role, "assistant");

    // Verify URL passthrough in OpenAI format
    let captured_body = captured.lock().await;
    let body = captured_body.as_ref().expect("mock should have captured a request");
    let content = body["messages"][0]["content"]
        .as_array()
        .expect("content should be an array");
    assert_eq!(content.len(), 2);

    assert_eq!(content[1]["type"], "image_url");
    assert_eq!(
        content[1]["image_url"]["url"],
        "https://example.com/photo.jpg"
    );
}

// ---------------------------------------------------------------------------
// E4-T3: Integration test with mixed content (text + image)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn mixed_text_and_image() {
    let (copilot_addr, captured, _h1) = spawn_capturing_mock_copilot().await;
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
                {"type": "text", "text": "Compare these two images:"},
                {
                    "type": "image",
                    "source": {
                        "type": "base64",
                        "media_type": "image/jpeg",
                        "data": "/9j/4AAQSkZJRg=="
                    }
                },
                {"type": "text", "text": "versus"},
                {
                    "type": "image",
                    "source": {
                        "type": "url",
                        "url": "https://example.com/second.png"
                    }
                }
            ]
        }]
    });

    let response = send_messages_request(app, body).await;

    assert_eq!(response.status(), StatusCode::OK);

    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let resp: AnthropicResponse = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(resp.response_type, "message");

    // Verify translated OpenAI request has all 4 blocks
    let captured_body = captured.lock().await;
    let body = captured_body.as_ref().unwrap();
    let content = body["messages"][0]["content"]
        .as_array()
        .expect("content should be an array");
    assert_eq!(content.len(), 4);

    // Block 0: text
    assert_eq!(content[0]["type"], "text");
    assert_eq!(content[0]["text"], "Compare these two images:");

    // Block 1: image_url (base64 → data URI)
    assert_eq!(content[1]["type"], "image_url");
    assert_eq!(
        content[1]["image_url"]["url"],
        "data:image/jpeg;base64,/9j/4AAQSkZJRg=="
    );

    // Block 2: text
    assert_eq!(content[2]["type"], "text");
    assert_eq!(content[2]["text"], "versus");

    // Block 3: image_url (URL passthrough)
    assert_eq!(content[3]["type"], "image_url");
    assert_eq!(
        content[3]["image_url"]["url"],
        "https://example.com/second.png"
    );
}

// ---------------------------------------------------------------------------
// E4-T4: Integration test with document block (verify skip)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn document_block_skipped() {
    let (copilot_addr, captured, _h1) = spawn_capturing_mock_copilot().await;
    let (github_addr, _h2) = spawn_mock_github().await;

    let state = create_test_state(
        format!("http://{copilot_addr}/chat/completions"),
        github_addr,
    )
    .await;
    let app = build_router(state);

    // Message with text + document (document should be skipped)
    let body = json!({
        "model": "claude-sonnet-4-20250514",
        "max_tokens": 1024,
        "messages": [{
            "role": "user",
            "content": [
                {"type": "text", "text": "Summarize this document:"},
                {
                    "type": "document",
                    "source": {
                        "type": "base64",
                        "media_type": "application/pdf",
                        "data": "JVBERi0xLjQ="
                    },
                    "title": "report.pdf"
                }
            ]
        }]
    });

    let response = send_messages_request(app, body).await;

    // Should succeed (not 422) — document blocks are accepted but skipped
    assert_eq!(response.status(), StatusCode::OK);

    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let resp: AnthropicResponse = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(resp.response_type, "message");

    // Verify document block was skipped in the OpenAI request — only text remains
    let captured_body = captured.lock().await;
    let body = captured_body.as_ref().unwrap();
    let content = body["messages"][0]["content"]
        .as_array()
        .expect("content should be an array");
    assert_eq!(content.len(), 1, "document block should be skipped");
    assert_eq!(content[0]["type"], "text");
    assert_eq!(content[0]["text"], "Summarize this document:");
}

// ---------------------------------------------------------------------------
// E4-T4 (supplement): Document-only message results in empty translation
// ---------------------------------------------------------------------------

#[tokio::test]
async fn document_only_message_handled_gracefully() {
    let (copilot_addr, _captured, _h1) = spawn_capturing_mock_copilot().await;
    let (github_addr, _h2) = spawn_mock_github().await;

    let state = create_test_state(
        format!("http://{copilot_addr}/chat/completions"),
        github_addr,
    )
    .await;
    let app = build_router(state);

    // A message with only a document block — after skip, no content remains.
    // The adapter should handle this gracefully (either empty messages → 400,
    // or the single message is dropped and upstream gets an empty messages array).
    let body = json!({
        "model": "claude-sonnet-4-20250514",
        "max_tokens": 1024,
        "messages": [{
            "role": "user",
            "content": [
                {
                    "type": "document",
                    "source": {
                        "type": "base64",
                        "media_type": "application/pdf",
                        "data": "JVBERi0xLjQ="
                    }
                }
            ]
        }]
    });

    let response = send_messages_request(app, body).await;

    // The adapter accepts the request (the Anthropic-level messages array is non-empty).
    // The document block is skipped during translation, resulting in an empty translated
    // messages array sent upstream. The mock returns 200 regardless, so we just verify
    // the request was accepted without a 422 deserialization error.
    let status = response.status();
    assert_ne!(
        status,
        StatusCode::UNPROCESSABLE_ENTITY,
        "document-only message must not cause a 422 deserialization error"
    );
}

// ---------------------------------------------------------------------------
// E4-T5: Verify mock Copilot receives correct OpenAI format
// ---------------------------------------------------------------------------

#[tokio::test]
async fn copilot_receives_correct_openai_format() {
    let (copilot_addr, captured, _h1) = spawn_capturing_mock_copilot().await;
    let (github_addr, _h2) = spawn_mock_github().await;

    let state = create_test_state(
        format!("http://{copilot_addr}/chat/completions"),
        github_addr,
    )
    .await;
    let app = build_router(state);

    let body = json!({
        "model": "claude-sonnet-4-20250514",
        "max_tokens": 2048,
        "system": "You are a vision assistant.",
        "messages": [{
            "role": "user",
            "content": [
                {"type": "text", "text": "Analyze this:"},
                {
                    "type": "image",
                    "source": {
                        "type": "base64",
                        "media_type": "image/webp",
                        "data": "UklGRg=="
                    }
                }
            ]
        }]
    });

    let response = send_messages_request(app, body).await;
    assert_eq!(response.status(), StatusCode::OK);

    let captured_body = captured.lock().await;
    let body = captured_body.as_ref().unwrap();

    // Verify top-level OpenAI request fields
    assert_eq!(body["model"], "claude-sonnet-4-20250514");
    assert_eq!(body["max_tokens"], 2048);
    // stream field is omitted when None (skip_serializing_if)
    assert!(
        body.get("stream").is_none() || body["stream"].is_null() || body["stream"] == false,
        "stream should be absent or false"
    );

    // Verify system message was prepended
    let messages = body["messages"].as_array().unwrap();
    assert_eq!(messages.len(), 2);
    assert_eq!(messages[0]["role"], "system");
    assert_eq!(messages[0]["content"], "You are a vision assistant.");

    // Verify user message has multimodal content blocks
    assert_eq!(messages[1]["role"], "user");
    let content = messages[1]["content"].as_array().unwrap();
    assert_eq!(content.len(), 2);

    assert_eq!(content[0]["type"], "text");
    assert_eq!(content[0]["text"], "Analyze this:");

    assert_eq!(content[1]["type"], "image_url");
    let image_url = &content[1]["image_url"];
    assert_eq!(image_url["url"], "data:image/webp;base64,UklGRg==");
    // detail field should be absent (null/not present)
    assert!(
        image_url.get("detail").is_none() || image_url["detail"].is_null(),
        "detail should not be set"
    );
}

// ---------------------------------------------------------------------------
// E4-T6: Verify response is valid Anthropic format
// ---------------------------------------------------------------------------

#[tokio::test]
async fn response_is_valid_anthropic_format() {
    let (copilot_addr, _captured, _h1) = spawn_capturing_mock_copilot().await;
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
                {"type": "text", "text": "What do you see?"},
                {
                    "type": "image",
                    "source": {
                        "type": "url",
                        "url": "https://example.com/landscape.jpg"
                    }
                }
            ]
        }]
    });

    let response = send_messages_request(app, body).await;
    assert_eq!(response.status(), StatusCode::OK);

    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();

    // Verify it parses as a valid AnthropicResponse
    let resp: AnthropicResponse = serde_json::from_slice(&bytes).unwrap();

    // Verify all required Anthropic response fields
    assert!(!resp.id.is_empty(), "id must be present");
    assert_eq!(resp.response_type, "message");
    assert_eq!(resp.role, "assistant");
    assert!(!resp.content.is_empty(), "content must not be empty");
    assert_eq!(resp.content[0].block_type(), "text");
    assert!(!resp.content[0].text_content().is_empty());
    assert_eq!(resp.stop_reason, Some("end_turn".to_string()));
    assert!(resp.usage.input_tokens > 0);
    assert!(resp.usage.output_tokens > 0);

    // Also verify raw JSON structure for strict format compliance
    let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(json["type"], "message");
    assert_eq!(json["role"], "assistant");
    assert!(json["content"].is_array());
    assert_eq!(json["content"][0]["type"], "text");
    assert!(json["usage"]["input_tokens"].is_number());
    assert!(json["usage"]["output_tokens"].is_number());
}

// ---------------------------------------------------------------------------
// E4-T7: Test with cache_control (verify accepted but not forwarded)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn cache_control_accepted_not_forwarded() {
    let (copilot_addr, captured, _h1) = spawn_capturing_mock_copilot().await;
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
                {
                    "type": "text",
                    "text": "Cached text content",
                    "cache_control": {"type": "ephemeral"}
                },
                {
                    "type": "image",
                    "source": {
                        "type": "base64",
                        "media_type": "image/png",
                        "data": "iVBORw0KGgo="
                    },
                    "cache_control": {"type": "ephemeral", "ttl": 300}
                }
            ]
        }]
    });

    let response = send_messages_request(app, body).await;

    // cache_control should not cause deserialization errors
    assert_eq!(response.status(), StatusCode::OK);

    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let resp: AnthropicResponse = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(resp.response_type, "message");

    // Verify cache_control is NOT forwarded to the OpenAI request
    let captured_body = captured.lock().await;
    let body = captured_body.as_ref().unwrap();
    let content = body["messages"][0]["content"].as_array().unwrap();

    // Text block should not have cache_control
    assert!(
        content[0].get("cache_control").is_none(),
        "cache_control should not be forwarded to OpenAI"
    );

    // Image block should not have cache_control
    assert!(
        content[1].get("cache_control").is_none(),
        "cache_control should not be forwarded to OpenAI image block"
    );
}

// ---------------------------------------------------------------------------
// Additional edge case: image with URL and media_type
// ---------------------------------------------------------------------------

#[tokio::test]
async fn image_url_with_media_type_passthrough() {
    let (copilot_addr, captured, _h1) = spawn_capturing_mock_copilot().await;
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
                {"type": "text", "text": "What is this?"},
                {
                    "type": "image",
                    "source": {
                        "type": "url",
                        "media_type": "image/jpeg",
                        "url": "https://cdn.example.com/photo.jpg"
                    }
                }
            ]
        }]
    });

    let response = send_messages_request(app, body).await;
    assert_eq!(response.status(), StatusCode::OK);

    // URL source should pass through the URL directly (media_type ignored for URL sources)
    let captured_body = captured.lock().await;
    let body = captured_body.as_ref().unwrap();
    let content = body["messages"][0]["content"].as_array().unwrap();
    assert_eq!(content[1]["image_url"]["url"], "https://cdn.example.com/photo.jpg");
}

// ---------------------------------------------------------------------------
// Additional edge case: mixed image + document (document skipped, image kept)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn mixed_image_and_document_keeps_image_skips_document() {
    let (copilot_addr, captured, _h1) = spawn_capturing_mock_copilot().await;
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
                {"type": "text", "text": "Look at this image and document:"},
                {
                    "type": "image",
                    "source": {
                        "type": "base64",
                        "media_type": "image/gif",
                        "data": "R0lGODlh"
                    }
                },
                {
                    "type": "document",
                    "source": {
                        "type": "base64",
                        "media_type": "application/pdf",
                        "data": "JVBERi0="
                    },
                    "title": "notes.pdf"
                }
            ]
        }]
    });

    let response = send_messages_request(app, body).await;
    assert_eq!(response.status(), StatusCode::OK);

    // Document should be skipped; text + image remain
    let captured_body = captured.lock().await;
    let body = captured_body.as_ref().unwrap();
    let content = body["messages"][0]["content"].as_array().unwrap();
    assert_eq!(content.len(), 2, "document block should be skipped, 2 blocks remain");

    assert_eq!(content[0]["type"], "text");
    assert_eq!(content[1]["type"], "image_url");
    assert_eq!(
        content[1]["image_url"]["url"],
        "data:image/gif;base64,R0lGODlh"
    );
}
