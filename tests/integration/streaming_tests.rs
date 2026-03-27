use std::sync::Arc;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::routing::post;
use axum::Router;
use serde_json::json;
use tokio::net::TcpListener;
use tower::ServiceExt;

use copilot_adapter::auth::device_flow::DeviceFlowAuth;
use copilot_adapter::auth::token::TokenManager;
use copilot_adapter::copilot::client::CopilotClient;
use copilot_adapter::copilot::types::ChatCompletionChunk;
use copilot_adapter::server::{build_router, AdapterConfig, AppState};

use super::test_helpers::InMemoryStorage;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Spawn a mock Copilot API that returns SSE streaming responses.
async fn spawn_mock_streaming_copilot(
) -> (std::net::SocketAddr, tokio::task::JoinHandle<()>) {
    let app = Router::new().route(
        "/chat/completions",
        post(mock_streaming_handler),
    );

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    let handle = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });

    (addr, handle)
}

/// Mock handler that returns 3 SSE chunks followed by [DONE].
async fn mock_streaming_handler(
    headers: axum::http::HeaderMap,
    axum::Json(body): axum::Json<serde_json::Value>,
) -> axum::response::Response {
    use axum::body::Body;
    use axum::http::Response;

    // Basic header validation
    let auth = headers
        .get("Authorization")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    if !auth.starts_with("Bearer ") {
        return Response::builder()
            .status(401)
            .body(Body::from("unauthorized"))
            .unwrap();
    }

    let model = body["model"].as_str().unwrap_or("gpt-4");

    // Build SSE chunks
    let chunks = vec![
        format!(
            "data: {}\n\n",
            json!({
                "id": "chatcmpl-stream1",
                "object": "chat.completion.chunk",
                "created": 1700000000,
                "model": model,
                "choices": [{"index": 0, "delta": {"role": "assistant"}, "finish_reason": null}]
            })
        ),
        format!(
            "data: {}\n\n",
            json!({
                "id": "chatcmpl-stream1",
                "object": "chat.completion.chunk",
                "created": 1700000000,
                "model": model,
                "choices": [{"index": 0, "delta": {"content": "Hello"}, "finish_reason": null}]
            })
        ),
        format!(
            "data: {}\n\n",
            json!({
                "id": "chatcmpl-stream1",
                "object": "chat.completion.chunk",
                "created": 1700000000,
                "model": model,
                "choices": [{"index": 0, "delta": {"content": " world!"}, "finish_reason": "stop"}]
            })
        ),
        "data: [DONE]\n\n".to_string(),
    ];

    let sse_body: String = chunks.concat();

    Response::builder()
        .status(200)
        .header("Content-Type", "text/event-stream")
        .body(Body::from(sse_body))
        .unwrap()
}

/// Spawn a mock GitHub server that provides Copilot tokens.
async fn spawn_mock_github_for_copilot_token(
) -> (std::net::SocketAddr, tokio::task::JoinHandle<()>) {
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
        config: AdapterConfig::default(),
    })
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
// Tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn streaming_returns_sse_events_in_order() {
    let (copilot_addr, _h1) = spawn_mock_streaming_copilot().await;
    let (github_addr, _h2) = spawn_mock_github_for_copilot_token().await;

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

    // Content-Type should indicate SSE
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

    // Should have 3 data chunks + final [DONE]
    assert_eq!(chunks.len(), 3, "Expected 3 chunks, body: {body_text}");
    assert!(saw_done, "Expected [DONE] marker in SSE body");

    // Verify chunk ordering and content
    assert_eq!(chunks[0].id, "chatcmpl-stream1");
    assert_eq!(chunks[0].choices[0].delta.role, Some("assistant".to_string()));

    assert_eq!(chunks[1].choices[0].delta.content, Some("Hello".to_string()));

    assert_eq!(chunks[2].choices[0].delta.content, Some(" world!".to_string()));
    assert_eq!(chunks[2].choices[0].finish_reason, Some("stop".to_string()));
}

#[tokio::test]
async fn streaming_chunks_have_data_prefix_and_valid_json() {
    let (copilot_addr, _h1) = spawn_mock_streaming_copilot().await;
    let (github_addr, _h2) = spawn_mock_github_for_copilot_token().await;

    let state = create_test_state(
        format!("http://{copilot_addr}/chat/completions"),
        github_addr,
    )
    .await;
    let app = build_router(state);

    let body = json!({
        "model": "gpt-4",
        "messages": [{"role": "user", "content": "Hi"}],
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

    // Every non-empty line should start with "data:" or be empty
    for frame in body_text.split("\n\n") {
        for line in frame.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with(':') {
                continue;
            }
            assert!(
                line.starts_with("data:"),
                "Expected line to start with 'data:', got: {line}"
            );
            // Parse the data payload — should be valid JSON or [DONE]
            let data = line.strip_prefix("data:").unwrap().trim();
            if data != "[DONE]" {
                let parsed: serde_json::Value = serde_json::from_str(data)
                    .unwrap_or_else(|e| panic!("Invalid JSON in SSE data: {e}\ndata: {data}"));
                assert!(parsed.get("id").is_some());
                assert_eq!(parsed["object"], "chat.completion.chunk");
            }
        }
    }
}

#[tokio::test]
async fn concurrent_streaming_requests_complete_successfully() {
    let (copilot_addr, _h1) = spawn_mock_streaming_copilot().await;
    let (github_addr, _h2) = spawn_mock_github_for_copilot_token().await;

    let state = create_test_state(
        format!("http://{copilot_addr}/chat/completions"),
        github_addr,
    )
    .await;

    // Start a real server to handle concurrent requests
    let app = build_router(state);
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let server_addr = listener.local_addr().unwrap();

    let server_handle = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });

    // Give server a moment to start
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;

    let client = reqwest::Client::new();

    // Launch 5 concurrent streaming requests
    let mut handles = Vec::new();
    for i in 0..5 {
        let client = client.clone();
        let url = format!("http://{server_addr}/v1/chat/completions");
        handles.push(tokio::spawn(async move {
            let body = json!({
                "model": "gpt-4",
                "messages": [{"role": "user", "content": format!("Hello {i}")}],
                "stream": true
            });

            let resp = client
                .post(&url)
                .header("Content-Type", "application/json")
                .json(&body)
                .send()
                .await
                .unwrap();

            assert_eq!(
                resp.status(),
                200,
                "Request {i} failed with status {}",
                resp.status()
            );

            let body_text = resp.text().await.unwrap();
            let (chunks, saw_done) = parse_sse_body_standalone(&body_text);

            assert!(
                chunks.len() >= 3,
                "Request {i} expected at least 3 chunks, got {}",
                chunks.len()
            );
            assert!(saw_done, "Request {i} missing [DONE] marker");

            i
        }));
    }

    // Collect results — all 5 should succeed
    let mut results = Vec::new();
    for handle in handles {
        results.push(handle.await.unwrap());
    }
    results.sort();
    assert_eq!(results, vec![0, 1, 2, 3, 4]);

    server_handle.abort();
}

/// Standalone SSE body parser (duplicated for concurrent test where we can't
/// reference the module-level helper from inside spawn).
fn parse_sse_body_standalone(body_text: &str) -> (Vec<ChatCompletionChunk>, bool) {
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

#[tokio::test]
async fn non_streaming_still_works_after_streaming_added() {
    // Regression: ensure non-streaming path is not broken
    let (_copilot_addr, _h1) = spawn_mock_streaming_copilot().await;
    let (github_addr, _h2) = spawn_mock_github_for_copilot_token().await;

    // For non-streaming, we need a mock that returns JSON (not SSE).
    // Reuse the streaming mock but send stream: false — the mock always
    // returns SSE format regardless, but our CopilotClient.send_chat_completion
    // will parse JSON. So we need a proper non-streaming mock.
    // We'll spawn a separate non-streaming mock.
    let non_stream_app = Router::new().route(
        "/chat/completions",
        post(|axum::Json(body): axum::Json<serde_json::Value>| async move {
            let model = body["model"].as_str().unwrap_or("gpt-4");
            axum::Json(json!({
                "id": "chatcmpl-nonstream",
                "object": "chat.completion",
                "created": 1700000000,
                "model": model,
                "choices": [{
                    "index": 0,
                    "message": {"role": "assistant", "content": "Non-streaming response"},
                    "finish_reason": "stop"
                }],
                "usage": {"prompt_tokens": 5, "completion_tokens": 3, "total_tokens": 8}
            }))
        }),
    );

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let ns_addr = listener.local_addr().unwrap();
    let _h3 = tokio::spawn(async move {
        axum::serve(listener, non_stream_app).await.unwrap();
    });

    let state = create_test_state(
        format!("http://{ns_addr}/chat/completions"),
        github_addr,
    )
    .await;
    let app = build_router(state);

    let body = json!({
        "model": "gpt-4",
        "messages": [{"role": "user", "content": "Hello"}],
        "stream": false
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
    let resp: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(resp["object"], "chat.completion");
    assert_eq!(resp["choices"][0]["message"]["content"], "Non-streaming response");
}
