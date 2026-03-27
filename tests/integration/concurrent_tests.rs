//! Integration tests for concurrent client handling.
//!
//! Verifies that the adapter can serve 10+ simultaneous requests correctly,
//! covering both streaming and non-streaming modes.

use std::sync::Arc;

use copilot_adapter::auth::device_flow::DeviceFlowAuth;
use copilot_adapter::auth::token::TokenManager;
use copilot_adapter::copilot::client::CopilotClient;
use copilot_adapter::copilot::models_cache::ModelsCache;
use copilot_adapter::copilot::types::ChatCompletionChunk;
use copilot_adapter::server::{build_router, AdapterConfig, AppState};
use serde_json::json;
use tokio::net::TcpListener;

#[path = "../common/mod.rs"]
mod common;
use common::mock_copilot::MockCopilot;
use common::mock_github::MockGitHub;

use super::test_helpers::InMemoryStorage;

/// Create test AppState pointing at mock servers with a pre-loaded token.
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

/// Parse raw SSE body text into chunks (standalone version for spawned tasks).
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

/// Spawn a real HTTP server backed by the adapter router.
async fn spawn_test_server(state: Arc<AppState>) -> (std::net::SocketAddr, tokio::task::JoinHandle<()>) {
    let app = build_router(state);
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let handle = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    // Give the server a moment to bind
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    (addr, handle)
}

#[tokio::test]
async fn ten_concurrent_non_streaming_requests_all_succeed() {
    let copilot = MockCopilot::spawn().await;
    let github = MockGitHub::spawn_copilot_token_only().await;

    let state = create_test_state(copilot.completions_url(), github.addr).await;
    let (server_addr, server_handle) = spawn_test_server(state).await;

    let client = reqwest::Client::new();
    let mut handles = Vec::new();

    for i in 0..10 {
        let client = client.clone();
        let url = format!("http://{server_addr}/v1/chat/completions");
        handles.push(tokio::spawn(async move {
            let body = json!({
                "model": "gpt-4",
                "messages": [{"role": "user", "content": format!("Concurrent request {i}")}]
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
                "Non-streaming request {i} failed with status {}",
                resp.status()
            );

            let json: serde_json::Value = resp.json().await.unwrap();
            assert_eq!(json["object"], "chat.completion");
            assert_eq!(json["choices"][0]["message"]["content"], "Hello from mock Copilot!");
            i
        }));
    }

    let mut results = Vec::new();
    for handle in handles {
        results.push(handle.await.unwrap());
    }
    results.sort();
    assert_eq!(results, (0..10).collect::<Vec<_>>());

    server_handle.abort();
}

#[tokio::test]
async fn ten_concurrent_streaming_requests_all_succeed() {
    let copilot = MockCopilot::spawn().await;
    let github = MockGitHub::spawn_copilot_token_only().await;

    let state = create_test_state(copilot.completions_url(), github.addr).await;
    let (server_addr, server_handle) = spawn_test_server(state).await;

    let client = reqwest::Client::new();
    let mut handles = Vec::new();

    for i in 0..10 {
        let client = client.clone();
        let url = format!("http://{server_addr}/v1/chat/completions");
        handles.push(tokio::spawn(async move {
            let body = json!({
                "model": "gpt-4",
                "messages": [{"role": "user", "content": format!("Stream request {i}")}],
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
                "Streaming request {i} failed with status {}",
                resp.status()
            );

            let body_text = resp.text().await.unwrap();
            let (chunks, saw_done) = parse_sse_body(&body_text);

            assert!(
                !chunks.is_empty(),
                "Streaming request {i} returned no chunks"
            );
            assert!(saw_done, "Streaming request {i} missing [DONE] marker");
            i
        }));
    }

    let mut results = Vec::new();
    for handle in handles {
        results.push(handle.await.unwrap());
    }
    results.sort();
    assert_eq!(results, (0..10).collect::<Vec<_>>());

    server_handle.abort();
}

#[tokio::test]
async fn mixed_streaming_and_non_streaming_concurrent_requests() {
    let copilot = MockCopilot::spawn().await;
    let github = MockGitHub::spawn_copilot_token_only().await;

    let state = create_test_state(copilot.completions_url(), github.addr).await;
    let (server_addr, server_handle) = spawn_test_server(state).await;

    let client = reqwest::Client::new();
    let mut handles = Vec::new();

    for i in 0..12 {
        let client = client.clone();
        let url = format!("http://{server_addr}/v1/chat/completions");
        let stream = i % 2 == 0; // Alternate streaming/non-streaming

        handles.push(tokio::spawn(async move {
            let body = json!({
                "model": "gpt-4",
                "messages": [{"role": "user", "content": format!("Mixed request {i}")}],
                "stream": stream
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
                "Mixed request {i} (stream={stream}) failed with status {}",
                resp.status()
            );

            if stream {
                let body_text = resp.text().await.unwrap();
                let (_chunks, saw_done) = parse_sse_body(&body_text);
                assert!(saw_done, "Stream request {i} missing [DONE]");
            } else {
                let json: serde_json::Value = resp.json().await.unwrap();
                assert_eq!(json["object"], "chat.completion");
            }

            i
        }));
    }

    let mut results = Vec::new();
    for handle in handles {
        results.push(handle.await.unwrap());
    }
    results.sort();
    assert_eq!(results, (0..12).collect::<Vec<_>>());

    server_handle.abort();
}

#[tokio::test]
async fn fifteen_concurrent_requests_with_slow_upstream() {
    // Slow upstream to simulate real-world latency; verifies the adapter
    // doesn't serialize requests.
    let copilot = MockCopilot::spawn_slow(50).await;
    let github = MockGitHub::spawn_copilot_token_only().await;

    let state = create_test_state(copilot.completions_url(), github.addr).await;
    let (server_addr, server_handle) = spawn_test_server(state).await;

    let client = reqwest::Client::new();
    let start = std::time::Instant::now();

    let mut handles = Vec::new();
    for i in 0..15 {
        let client = client.clone();
        let url = format!("http://{server_addr}/v1/chat/completions");
        handles.push(tokio::spawn(async move {
            let body = json!({
                "model": "gpt-4",
                "messages": [{"role": "user", "content": format!("Slow request {i}")}]
            });

            let resp = client
                .post(&url)
                .header("Content-Type", "application/json")
                .json(&body)
                .send()
                .await
                .unwrap();

            assert_eq!(resp.status(), 200, "Slow request {i} failed");
            i
        }));
    }

    for handle in handles {
        handle.await.unwrap();
    }

    let elapsed = start.elapsed();
    // If requests were serialized, 15 × 50ms = 750ms minimum.
    // Concurrent execution should take much less than that.
    assert!(
        elapsed.as_millis() < 700,
        "Requests appear to be serialized: elapsed {}ms (expected <700ms for 15 concurrent 50ms requests)",
        elapsed.as_millis()
    );

    server_handle.abort();
}

#[tokio::test]
async fn concurrent_requests_to_health_and_models_endpoints() {
    let copilot = MockCopilot::spawn().await;
    let github = MockGitHub::spawn_copilot_token_only().await;

    let state = create_test_state(copilot.completions_url(), github.addr).await;
    let (server_addr, server_handle) = spawn_test_server(state).await;

    let client = reqwest::Client::new();
    let mut handles = Vec::new();

    // Mix health, models, and chat requests
    for i in 0..10 {
        let client = client.clone();
        let base = format!("http://{server_addr}");

        handles.push(tokio::spawn(async move {
            let resp = match i % 3 {
                0 => {
                    // Health check
                    client.get(format!("{base}/health")).send().await.unwrap()
                }
                1 => {
                    // Models list
                    client.get(format!("{base}/v1/models")).send().await.unwrap()
                }
                _ => {
                    // Chat completion
                    let body = json!({
                        "model": "gpt-4",
                        "messages": [{"role": "user", "content": "Hello"}]
                    });
                    client
                        .post(format!("{base}/v1/chat/completions"))
                        .header("Content-Type", "application/json")
                        .json(&body)
                        .send()
                        .await
                        .unwrap()
                }
            };

            assert_eq!(
                resp.status(),
                200,
                "Request {i} (endpoint={}) failed with status {}",
                i % 3,
                resp.status()
            );
            i
        }));
    }

    let mut results = Vec::new();
    for handle in handles {
        results.push(handle.await.unwrap());
    }
    results.sort();
    assert_eq!(results, (0..10).collect::<Vec<_>>());

    server_handle.abort();
}
