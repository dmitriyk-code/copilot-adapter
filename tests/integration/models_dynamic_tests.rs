//! Integration tests for dynamic models fetching (Epics 3 & 5).
//!
//! Covers:
//! - Cache hit returns cached data without API call
//! - Cache miss fetches from mock Copilot API
//! - API error triggers fallback to static model list
//! - Static models mode always uses fallback
//! - get_model with valid/invalid IDs
//! - TTL expiry triggers refetch from API
//! - Request counter verifies cache prevents duplicate calls

use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Duration;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use tower::ServiceExt;

use copilot_adapter::auth::device_flow::DeviceFlowAuth;
use copilot_adapter::auth::token::TokenManager;
use copilot_adapter::copilot::client::CopilotClient;
use copilot_adapter::copilot::models_cache::ModelsCache;
use copilot_adapter::copilot::types::{Model, ModelList};
use copilot_adapter::server::{build_router, AdapterConfig, AppState};

use super::test_helpers::InMemoryStorage;
use crate::common::mock_copilot::MockCopilotModels;
use crate::common::mock_github::MockGitHub;

// ---------------------------------------------------------------------------
// State builders (using reusable mocks from tests/common/)
// ---------------------------------------------------------------------------

/// Build AppState wired to reusable mock servers with dynamic models enabled.
async fn create_dynamic_state(
    models_url: String,
    github: &MockGitHub,
    cache_ttl: Duration,
) -> Arc<AppState> {
    let auth = DeviceFlowAuth::with_urls(
        format!("http://{}/unused", github.addr),
        format!("http://{}/unused", github.addr),
        github.copilot_token_url(),
    );
    let storage = InMemoryStorage::with_token("test_github_token");
    let tm = Arc::new(TokenManager::new(Box::new(storage), auth).await.unwrap());
    let client = reqwest::Client::new();

    Arc::new(AppState {
        token_manager: tm,
        copilot_client: CopilotClient::with_api_url(client.clone(), "http://localhost:1/unused".into())
            .with_models_url(models_url),
        http_client: client,
        config: AdapterConfig {
            static_models: false,
            ..AdapterConfig::default()
        },
        models_cache: ModelsCache::new(cache_ttl),
    })
}

/// Build AppState with --static-models enabled (no API calls).
async fn create_static_state() -> Arc<AppState> {
    let auth = DeviceFlowAuth::with_urls(
        "http://localhost:1/unused".into(),
        "http://localhost:1/unused".into(),
        "http://localhost:1/unused".into(),
    );
    let storage = InMemoryStorage::new();
    let tm = Arc::new(TokenManager::new(Box::new(storage), auth).await.unwrap());
    let client = reqwest::Client::new();

    Arc::new(AppState {
        token_manager: tm,
        copilot_client: CopilotClient::new(client.clone()),
        http_client: client,
        config: AdapterConfig {
            static_models: true,
            ..AdapterConfig::default()
        },
        models_cache: ModelsCache::new(Duration::from_secs(300)),
    })
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

/// Cache hit returns cached data without making an API call.
#[tokio::test]
async fn cache_hit_returns_cached_data() {
    let github = MockGitHub::spawn_copilot_token_only().await;
    let mock_models = MockCopilotModels::spawn().await;

    let state = create_dynamic_state(
        mock_models.models_url(),
        &github,
        Duration::from_secs(300),
    )
    .await;

    // Pre-populate cache with custom data.
    let cached_list = ModelList {
        object: "list".to_string(),
        data: vec![Model {
            id: "cached-model".to_string(),
            object: "model".to_string(),
            created: 1000000,
            owned_by: "test".to_string(),
        }],
    };
    state.models_cache.set(cached_list).await;

    let app = build_router(state);
    let response = app
        .oneshot(
            Request::builder()
                .uri("/v1/models")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);

    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let list: ModelList = serde_json::from_slice(&bytes).unwrap();

    assert_eq!(list.data.len(), 1);
    assert_eq!(list.data[0].id, "cached-model");
}

/// Cache miss fetches models from the mock Copilot API.
#[tokio::test]
async fn cache_miss_fetches_from_api() {
    let github = MockGitHub::spawn_copilot_token_only().await;
    let mock_models = MockCopilotModels::spawn().await;

    let state = create_dynamic_state(
        mock_models.models_url(),
        &github,
        Duration::from_secs(300),
    )
    .await;

    let app = build_router(state);
    let response = app
        .oneshot(
            Request::builder()
                .uri("/v1/models")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);

    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let list: ModelList = serde_json::from_slice(&bytes).unwrap();

    // Should contain the dynamic models from the reusable MockCopilotModels.
    let ids: Vec<&str> = list.data.iter().map(|m| m.id.as_str()).collect();
    assert!(ids.contains(&"gpt-4o"), "Should contain gpt-4o from mock API");
    assert!(ids.contains(&"gpt-4"), "Should contain gpt-4 from mock API");
    assert!(ids.contains(&"claude-sonnet-4"), "Should contain claude-sonnet-4 from mock API");
    assert_eq!(list.data.len(), 3, "Mock API returns exactly 3 models");
}

/// API error triggers fallback to the static model list.
#[tokio::test]
async fn api_error_triggers_fallback() {
    let github = MockGitHub::spawn_copilot_token_only().await;
    let mock_models = MockCopilotModels::spawn_failing().await;

    let state = create_dynamic_state(
        mock_models.models_url(),
        &github,
        Duration::from_secs(300),
    )
    .await;

    let app = build_router(state);
    let response = app
        .oneshot(
            Request::builder()
                .uri("/v1/models")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);

    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let list: ModelList = serde_json::from_slice(&bytes).unwrap();

    let ids: Vec<&str> = list.data.iter().map(|m| m.id.as_str()).collect();
    assert!(ids.contains(&"gpt-4o"), "Fallback should contain gpt-4o");
    assert!(ids.contains(&"gpt-4"), "Fallback should contain gpt-4");
    assert!(ids.contains(&"gpt-3.5-turbo"), "Fallback should contain gpt-3.5-turbo");
    assert!(!ids.contains(&"claude-3.5-sonnet"), "Fallback should not contain retired claude-3.5-sonnet");
}

/// Static models mode always uses fallback without API calls.
#[tokio::test]
async fn static_mode_uses_fallback() {
    let state = create_static_state().await;

    let app = build_router(state);
    let response = app
        .oneshot(
            Request::builder()
                .uri("/v1/models")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);

    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let list: ModelList = serde_json::from_slice(&bytes).unwrap();

    let ids: Vec<&str> = list.data.iter().map(|m| m.id.as_str()).collect();
    assert!(ids.contains(&"gpt-4o"), "Should contain gpt-4o");
    assert!(ids.contains(&"gpt-4"), "Should contain gpt-4");
    assert!(ids.contains(&"gpt-4-turbo"), "Should contain gpt-4-turbo");
    assert!(ids.contains(&"gpt-3.5-turbo"), "Should contain gpt-3.5-turbo");
    assert_eq!(list.data.len(), 4, "Fallback list should have exactly 4 models");
}

/// get_model with a valid model ID succeeds.
#[tokio::test]
async fn get_model_valid_id_succeeds() {
    let github = MockGitHub::spawn_copilot_token_only().await;
    let mock_models = MockCopilotModels::spawn().await;

    let state = create_dynamic_state(
        mock_models.models_url(),
        &github,
        Duration::from_secs(300),
    )
    .await;

    let app = build_router(state);
    let response = app
        .oneshot(
            Request::builder()
                .uri("/v1/models/gpt-4o")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);

    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let model: Model = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(model.id, "gpt-4o");
    assert_eq!(model.object, "model");
}

/// get_model with an invalid model ID returns 404.
#[tokio::test]
async fn get_model_invalid_id_returns_404() {
    let github = MockGitHub::spawn_copilot_token_only().await;
    let mock_models = MockCopilotModels::spawn().await;

    let state = create_dynamic_state(
        mock_models.models_url(),
        &github,
        Duration::from_secs(300),
    )
    .await;

    let app = build_router(state);
    let response = app
        .oneshot(
            Request::builder()
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
    assert!(
        json["error"]["message"]
            .as_str()
            .unwrap()
            .contains("not found"),
        "Error message should mention 'not found'"
    );
}

/// Request counter verifies cache prevents duplicate API calls.
#[tokio::test]
async fn cache_prevents_duplicate_api_calls() {
    let github = MockGitHub::spawn_copilot_token_only().await;
    let (mock_models, counter) = MockCopilotModels::spawn_with_counter().await;

    let state = create_dynamic_state(
        mock_models.models_url(),
        &github,
        Duration::from_secs(300),
    )
    .await;

    // First request: cache miss -> API call.
    let app = build_router(state.clone());
    let response = app
        .oneshot(
            Request::builder()
                .uri("/v1/models")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(counter.load(Ordering::SeqCst), 1, "First request should call API");

    // Second request: cache hit -> no API call.
    let app = build_router(state.clone());
    let response = app
        .oneshot(
            Request::builder()
                .uri("/v1/models")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(counter.load(Ordering::SeqCst), 1, "Second request should use cache");

    // Third request: still cached.
    let app = build_router(state);
    let response = app
        .oneshot(
            Request::builder()
                .uri("/v1/models")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(counter.load(Ordering::SeqCst), 1, "Third request should still use cache");
}

/// TTL expiry causes a re-fetch from the API.
#[tokio::test]
async fn ttl_expiry_causes_refetch() {
    let github = MockGitHub::spawn_copilot_token_only().await;
    let (mock_models, counter) = MockCopilotModels::spawn_with_counter().await;

    // Very short TTL so we can test expiry without long sleeps.
    let state = create_dynamic_state(
        mock_models.models_url(),
        &github,
        Duration::from_millis(100),
    )
    .await;

    // First request: cache miss -> API call.
    let app = build_router(state.clone());
    let response = app
        .oneshot(
            Request::builder()
                .uri("/v1/models")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(counter.load(Ordering::SeqCst), 1, "First request should call API");

    // Wait for TTL to expire.
    tokio::time::sleep(Duration::from_millis(150)).await;

    // Second request: cache expired -> API call again.
    let app = build_router(state);
    let response = app
        .oneshot(
            Request::builder()
                .uri("/v1/models")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(counter.load(Ordering::SeqCst), 2, "After TTL expiry, should call API again");
}
