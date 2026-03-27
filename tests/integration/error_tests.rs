use std::sync::Arc;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::response::IntoResponse;
use axum::routing::post;
use axum::{Json, Router};
use serde_json::json;
use tokio::net::TcpListener;
use tower::ServiceExt;

use copilot_adapter::auth::device_flow::DeviceFlowAuth;
use copilot_adapter::auth::token::TokenManager;
use copilot_adapter::copilot::client::CopilotClient;
use copilot_adapter::server::{build_router, AdapterConfig, AppState};

use super::test_helpers::InMemoryStorage;

/// Spawn a mock GitHub server that returns Copilot tokens.
async fn spawn_mock_github(
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

/// Create test AppState pointing at mock backends.
async fn create_test_state(
    copilot_api_url: String,
    github_addr: std::net::SocketAddr,
    with_token: bool,
) -> Arc<AppState> {
    let auth = DeviceFlowAuth::with_urls(
        format!("http://{github_addr}/unused"),
        format!("http://{github_addr}/unused"),
        format!("http://{github_addr}/copilot_internal/v2/token"),
    );

    let storage = if with_token {
        InMemoryStorage::with_token("test_github_token")
    } else {
        InMemoryStorage::new()
    };

    let tm = Arc::new(TokenManager::new(Box::new(storage), auth).await.unwrap());
    let client = reqwest::Client::new();

    Arc::new(AppState {
        token_manager: tm,
        copilot_client: CopilotClient::with_api_url(client.clone(), copilot_api_url),
        http_client: client,
        config: AdapterConfig::default(),
    })
}

/// Spawn a mock Copilot API that always returns 200.
async fn spawn_mock_copilot_ok() -> (std::net::SocketAddr, tokio::task::JoinHandle<()>) {
    let app = Router::new().route(
        "/chat/completions",
        post(|| async {
            Json(json!({
                "id": "chatcmpl-ok",
                "object": "chat.completion",
                "created": 1700000000,
                "model": "gpt-4",
                "choices": [{
                    "index": 0,
                    "message": {"role": "assistant", "content": "Hello"},
                    "finish_reason": "stop"
                }],
                "usage": {"prompt_tokens": 5, "completion_tokens": 1, "total_tokens": 6}
            }))
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
async fn invalid_request_returns_400_with_openai_error_json() {
    let (copilot_addr, _h1) = spawn_mock_copilot_ok().await;
    let (github_addr, _h2) = spawn_mock_github().await;

    let state = create_test_state(
        format!("http://{copilot_addr}/chat/completions"),
        github_addr,
        true,
    )
    .await;
    let app = build_router(state);

    // Send request with empty messages array (invalid)
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

    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();

    // Verify OpenAI-compatible error format
    assert!(json.get("error").is_some(), "must have 'error' field");
    assert_eq!(json["error"]["type"], "invalid_request_error");
    assert_eq!(json["error"]["code"], "invalid_request");
    assert!(json["error"]["message"].as_str().unwrap().len() > 0);
}

#[tokio::test]
async fn auth_failure_returns_401_with_openai_error_json() {
    let (copilot_addr, _h1) = spawn_mock_copilot_ok().await;
    let (github_addr, _h2) = spawn_mock_github().await;

    // Create state with NO GitHub token (unauthenticated)
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

    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);

    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();

    // Verify OpenAI-compatible error format
    assert!(json.get("error").is_some(), "must have 'error' field");
    assert_eq!(json["error"]["type"], "authentication_error");
    assert_eq!(json["error"]["code"], "not_authenticated");
}

#[tokio::test]
async fn not_found_model_returns_404_with_openai_error_json() {
    let (copilot_addr, _h1) = spawn_mock_copilot_ok().await;
    let (github_addr, _h2) = spawn_mock_github().await;

    let state = create_test_state(
        format!("http://{copilot_addr}/chat/completions"),
        github_addr,
        true,
    )
    .await;
    let app = build_router(state);

    let response = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/v1/models/nonexistent-model")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::NOT_FOUND);

    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();

    assert!(json.get("error").is_some(), "must have 'error' field");
    assert_eq!(json["error"]["type"], "not_found_error");
    assert_eq!(json["error"]["code"], "model_not_found");
}

#[tokio::test(start_paused = true)]
async fn upstream_error_returns_502_with_openai_error_json() {
    // Copilot API that returns 500
    let app = Router::new().route(
        "/chat/completions",
        post(|| async {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({"error": "server error"})),
            )
        }),
    );

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let copilot_addr = listener.local_addr().unwrap();
    let _h1 = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });

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

    assert_eq!(response.status(), StatusCode::BAD_GATEWAY);

    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();

    assert!(json.get("error").is_some(), "must have 'error' field");
    assert_eq!(json["error"]["type"], "upstream_error");
    assert_eq!(json["error"]["code"], "copilot_error");
}

#[tokio::test]
async fn rate_limited_returns_429_with_retry_after_header() {
    // Copilot API that returns 429 with Retry-After
    let app = Router::new().route(
        "/chat/completions",
        post(|| async {
            let mut response = (
                StatusCode::TOO_MANY_REQUESTS,
                Json(json!({"error": "rate limited"})),
            )
                .into_response();
            response.headers_mut().insert(
                "Retry-After",
                axum::http::HeaderValue::from_static("42"),
            );
            response
        }),
    );

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let copilot_addr = listener.local_addr().unwrap();
    let _h1 = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });

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

    assert_eq!(response.status(), StatusCode::TOO_MANY_REQUESTS);

    // Verify Retry-After header is forwarded
    let retry_after = response
        .headers()
        .get("Retry-After")
        .and_then(|v| v.to_str().ok())
        .unwrap();
    assert_eq!(retry_after, "42");

    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();

    assert_eq!(json["error"]["type"], "rate_limit_error");
    assert_eq!(json["error"]["code"], "rate_limited");
    assert_eq!(json["error"]["retry_after"], 42);
}

#[tokio::test]
async fn response_includes_x_request_id_header() {
    let (copilot_addr, _h1) = spawn_mock_copilot_ok().await;
    let (github_addr, _h2) = spawn_mock_github().await;

    let state = create_test_state(
        format!("http://{copilot_addr}/chat/completions"),
        github_addr,
        true,
    )
    .await;
    let app = build_router(state);

    let response = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/health")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);

    // Request tracing middleware should inject X-Request-Id
    let request_id = response.headers().get("X-Request-Id");
    assert!(
        request_id.is_some(),
        "Response must include X-Request-Id header"
    );
    let id_str = request_id.unwrap().to_str().unwrap();
    assert!(!id_str.is_empty(), "X-Request-Id must not be empty");
}
