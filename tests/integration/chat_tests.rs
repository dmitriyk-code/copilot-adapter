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
use copilot_adapter::copilot::types::ChatCompletionResponse;
use copilot_adapter::server::{build_router, AdapterConfig, AppState};

use super::test_helpers::InMemoryStorage;

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

/// Mock chat completions endpoint that validates headers and returns a response.
/// Supports both streaming and non-streaming based on request body.
async fn mock_chat_completions(
    headers: axum::http::HeaderMap,
    Json(body): Json<serde_json::Value>,
) -> Result<Response, (StatusCode, Json<serde_json::Value>)> {
    // Validate Authorization header
    let auth = headers
        .get("Authorization")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    if !auth.starts_with("Bearer ") {
        return Err((
            StatusCode::UNAUTHORIZED,
            Json(json!({"error": "missing Bearer token"})),
        ));
    }

    // Validate required Copilot headers
    let integration_id = headers
        .get("Copilot-Integration-Id")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    if integration_id != "vscode-chat" {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(json!({"error": format!("expected Copilot-Integration-Id 'vscode-chat', got '{integration_id}'")})),
        ));
    }

    let editor_version = headers
        .get("Editor-Version")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    if editor_version != "vscode/1.85.0" {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(json!({"error": format!("expected Editor-Version 'vscode/1.85.0', got '{editor_version}'")})),
        ));
    }

    if headers.get("X-Request-Id").is_none() {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(json!({"error": "missing X-Request-Id header"})),
        ));
    }

    // Build response based on request
    let model = body["model"].as_str().unwrap_or("gpt-4");
    let stream = body["stream"].as_bool().unwrap_or(false);

    if stream {
        // Return SSE streaming response
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
                    "choices": [{"index": 0, "delta": {"content": "Hello from mock Copilot!"}, "finish_reason": "stop"}]
                })
            ),
            "data: [DONE]\n\n".to_string(),
        ];

        let sse_body: String = chunks.concat();

        return Ok(axum::http::Response::builder()
            .status(200)
            .header("Content-Type", "text/event-stream")
            .body(Body::from(sse_body))
            .unwrap()
            .into_response());
    }

    Ok(Json(json!({
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
    })).into_response())
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
                Json(json!({
                    "token": "test_copilot_token",
                    "expires_at": expires_at
                }))
            } else {
                // Return valid JSON that is missing the required `token` and
                // `expires_at` fields, causing CopilotToken deserialization to
                // fail and surfacing an auth error upstream.
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

/// Create test AppState with mock Copilot API and pre-loaded GitHub token.
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
        models_cache: ModelsCache::new(std::time::Duration::from_secs(300)),
    })
}

#[tokio::test]
async fn chat_completion_non_streaming_returns_response() {
    let (copilot_addr, _h1) = spawn_mock_copilot_api().await;
    let (github_addr, _h2) = spawn_mock_github_for_copilot_token().await;

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

    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let resp: ChatCompletionResponse = serde_json::from_slice(&bytes).unwrap();

    assert_eq!(resp.id, "chatcmpl-mock123");
    assert_eq!(resp.object, "chat.completion");
    assert_eq!(resp.model, "gpt-4");
    assert_eq!(resp.choices.len(), 1);
    assert_eq!(resp.choices[0].message.role, "assistant");
    assert_eq!(resp.choices[0].message.content.as_text(), "Hello from mock Copilot!");
    assert_eq!(resp.choices[0].finish_reason, Some("stop".to_string()));
    assert!(resp.usage.is_some());
}

#[tokio::test]
async fn chat_completion_with_stream_true_returns_sse() {
    let (copilot_addr, _h1) = spawn_mock_copilot_api().await;
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
    assert!(body_text.contains("data:"), "SSE body should contain data: lines");
    assert!(body_text.contains("[DONE]"), "SSE body should end with [DONE]");
}

#[tokio::test]
async fn chat_completion_empty_messages_returns_400() {
    let (copilot_addr, _h1) = spawn_mock_copilot_api().await;
    let (github_addr, _h2) = spawn_mock_github_for_copilot_token().await;

    let state = create_test_state(
        format!("http://{copilot_addr}/chat/completions"),
        github_addr,
    )
    .await;
    let app = build_router(state);

    let body = json!({
        "model": "gpt-4",
        "messages": []
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

#[tokio::test]
async fn chat_completion_invalid_json_returns_422() {
    let (copilot_addr, _h1) = spawn_mock_copilot_api().await;
    let (github_addr, _h2) = spawn_mock_github_for_copilot_token().await;

    let state = create_test_state(
        format!("http://{copilot_addr}/chat/completions"),
        github_addr,
    )
    .await;
    let app = build_router(state);

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/chat/completions")
                .header("Content-Type", "application/json")
                .body(Body::from("not json"))
                .unwrap(),
        )
        .await
        .unwrap();

    // axum returns 400 for JSON deserialization failures
    assert!(
        response.status() == StatusCode::UNPROCESSABLE_ENTITY
            || response.status() == StatusCode::BAD_REQUEST,
        "Expected 400 or 422 for invalid JSON, got {}",
        response.status()
    );
}

#[tokio::test]
async fn chat_completion_without_auth_returns_401() {
    let (copilot_addr, _h1) = spawn_mock_copilot_api().await;

    // Create state with NO GitHub token stored
    let auth = DeviceFlowAuth::with_urls(
        "http://localhost:1/unused".into(),
        "http://localhost:1/unused".into(),
        "http://localhost:1/unused".into(),
    );
    let storage = InMemoryStorage::new();
    let tm = Arc::new(TokenManager::new(Box::new(storage), auth).await.unwrap());
    let client = reqwest::Client::new();

    let state = Arc::new(AppState {
        token_manager: tm,
        copilot_client: CopilotClient::with_api_url(
            client.clone(),
            format!("http://{copilot_addr}/chat/completions"),
        ),
        http_client: client,
        config: AdapterConfig::default(),
        models_cache: ModelsCache::new(std::time::Duration::from_secs(300)),
    });
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

    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn chat_completion_response_matches_openai_format() {
    let (copilot_addr, _h1) = spawn_mock_copilot_api().await;
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
    let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();

    // Verify OpenAI response format fields
    assert!(json.get("id").is_some(), "response must have 'id'");
    assert_eq!(json["object"], "chat.completion");
    assert!(json.get("created").is_some(), "response must have 'created'");
    assert!(json.get("model").is_some(), "response must have 'model'");
    assert!(json.get("choices").is_some(), "response must have 'choices'");
    assert!(json["choices"].is_array());
    assert!(json.get("usage").is_some(), "response must have 'usage'");
}

/// Spawn a mock Copilot API that always returns an HTTP 500 error.
async fn spawn_failing_copilot_api() -> (std::net::SocketAddr, tokio::task::JoinHandle<()>) {
    let app = Router::new().route(
        "/chat/completions",
        post(|| async {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({"error": "internal server error"})),
            )
        }),
    );

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    let handle = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });

    (addr, handle)
}

#[tokio::test(start_paused = true)]
async fn chat_completion_upstream_error_returns_502() {
    let (copilot_addr, _h1) = spawn_failing_copilot_api().await;
    let (github_addr, _h2) = spawn_mock_github_for_copilot_token().await;

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

    assert_eq!(response.status(), StatusCode::BAD_GATEWAY);

    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(json["error"]["type"], "upstream_error");
}
