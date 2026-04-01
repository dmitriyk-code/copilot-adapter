use std::sync::Arc;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use tower::ServiceExt;

use copilot_adapter::auth::device_flow::DeviceFlowAuth;
use copilot_adapter::auth::token::TokenManager;
use copilot_adapter::copilot::client::CopilotClient;
use copilot_adapter::copilot::models_cache::ModelsCache;
use copilot_adapter::server::{build_router, AdapterConfig, AppState};

use super::test_helpers::InMemoryStorage;

async fn test_state() -> Arc<AppState> {
    let auth = DeviceFlowAuth::with_urls(
        "http://localhost:1/unused".into(),
        "http://localhost:1/unused".into(),
        "http://localhost:1/unused".into(),
    );
    let tm = Arc::new(
        TokenManager::new(Box::new(InMemoryStorage::new()), auth)
            .await
            .unwrap(),
    );
    let client = reqwest::Client::new();
    Arc::new(AppState {
        token_manager: tm,
        copilot_client: CopilotClient::new(client),
        config: AdapterConfig::default(),
        models_cache: ModelsCache::new(std::time::Duration::from_secs(300)),
        conversation_logger: None,
    })
}

#[tokio::test]
async fn count_tokens_valid_request_returns_200() {
    let state = test_state().await;
    let app = build_router(state);

    let body = serde_json::json!({
        "model": "claude-sonnet-4-20250514",
        "messages": [{"role": "user", "content": "Hello!"}]
    });

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/messages/count_tokens")
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

    assert!(json.get("input_tokens").is_some(), "Response must contain input_tokens");
    let input_tokens = json["input_tokens"].as_u64().unwrap();
    assert!(input_tokens > 0, "Token count should be > 0, got {input_tokens}");
}

#[tokio::test]
async fn count_tokens_missing_model_returns_400() {
    let state = test_state().await;
    let app = build_router(state);

    // Missing required "model" field
    let body = serde_json::json!({
        "messages": [{"role": "user", "content": "Hello!"}]
    });

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/messages/count_tokens")
                .header("Content-Type", "application/json")
                .body(Body::from(serde_json::to_vec(&body).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();

    // Deserialization failure results in 422 (Unprocessable Entity) from axum's
    // Json extractor, which is the correct behavior for missing required fields.
    assert!(
        response.status() == StatusCode::BAD_REQUEST
            || response.status() == StatusCode::UNPROCESSABLE_ENTITY,
        "Expected 400 or 422, got {}",
        response.status()
    );
}

#[tokio::test]
async fn count_tokens_empty_messages_returns_count() {
    let state = test_state().await;
    let app = build_router(state);

    let body = serde_json::json!({
        "model": "claude-sonnet-4-20250514",
        "messages": []
    });

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/messages/count_tokens")
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

    assert!(json.get("input_tokens").is_some(), "Response must contain input_tokens");
    let input_tokens = json["input_tokens"].as_u64().unwrap();
    // Empty messages should return 0 tokens
    assert_eq!(input_tokens, 0, "Empty messages should give 0 tokens, got {input_tokens}");
}

#[tokio::test]
async fn count_tokens_invalid_json_returns_error() {
    let state = test_state().await;
    let app = build_router(state);

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/messages/count_tokens")
                .header("Content-Type", "application/json")
                .body(Body::from("not json"))
                .unwrap(),
        )
        .await
        .unwrap();

    assert!(
        response.status().is_client_error(),
        "Invalid JSON should return 4xx, got {}",
        response.status()
    );
}

#[tokio::test]
async fn count_tokens_with_system_prompt() {
    let state = test_state().await;
    let app = build_router(state);

    let body = serde_json::json!({
        "model": "claude-sonnet-4-20250514",
        "system": "You are a helpful assistant.",
        "messages": [{"role": "user", "content": "Hello!"}]
    });

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/messages/count_tokens")
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

    let input_tokens = json["input_tokens"].as_u64().unwrap();
    // System prompt + message should be more than just the message alone
    assert!(input_tokens > 5, "System + message should be > 5 tokens, got {input_tokens}");
}
