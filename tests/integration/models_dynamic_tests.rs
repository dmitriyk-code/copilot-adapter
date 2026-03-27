//! Integration tests for dynamic models fetching (Epic 3).
//!
//! Covers:
//! - Cache hit returns cached data without API call
//! - Cache miss fetches from mock Copilot API
//! - API error triggers fallback to static model list
//! - Static models mode always uses fallback
//! - get_model with valid/invalid IDs

use std::sync::Arc;
use std::time::Duration;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::routing::get;
use axum::{Json, Router};
use serde_json::json;
use tokio::net::TcpListener;
use tower::ServiceExt;

use copilot_adapter::auth::device_flow::DeviceFlowAuth;
use copilot_adapter::auth::token::TokenManager;
use copilot_adapter::copilot::client::CopilotClient;
use copilot_adapter::copilot::models_cache::ModelsCache;
use copilot_adapter::copilot::types::{Model, ModelList};
use copilot_adapter::server::{build_router, AdapterConfig, AppState};

use super::test_helpers::InMemoryStorage;

// ---------------------------------------------------------------------------
// Mock servers
// ---------------------------------------------------------------------------

/// Spawn a mock GitHub API that issues Copilot tokens.
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

/// Spawn a mock Copilot models API that returns a custom model list.
async fn spawn_mock_models_api() -> (std::net::SocketAddr, tokio::task::JoinHandle<()>) {
    let app = Router::new().route(
        "/models",
        get(|headers: axum::http::HeaderMap| async move {
            // Validate bearer token
            let auth = headers
                .get("Authorization")
                .and_then(|v| v.to_str().ok())
                .unwrap_or("");
            if !auth.starts_with("Bearer ") {
                return (
                    StatusCode::UNAUTHORIZED,
                    Json(json!({"error": "missing token"})),
                )
                    .into_response();
            }

            let models = json!({
                "object": "list",
                "data": [
                    {
                        "id": "gpt-4o-dynamic",
                        "object": "model",
                        "created": 1715367049,
                        "owned_by": "github-copilot"
                    },
                    {
                        "id": "claude-sonnet-4",
                        "object": "model",
                        "created": 1715367049,
                        "owned_by": "github-copilot"
                    }
                ]
            });
            (StatusCode::OK, Json(models)).into_response()
        }),
    );

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let handle = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    (addr, handle)
}

/// Spawn a mock Copilot models API that always returns 500.
async fn spawn_failing_models_api() -> (std::net::SocketAddr, tokio::task::JoinHandle<()>) {
    let app = Router::new().route(
        "/models",
        get(|| async {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({"error": "internal error"})),
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

/// Build AppState wired to mock servers with dynamic models enabled.
async fn create_dynamic_state(
    models_api_url: String,
    github_addr: std::net::SocketAddr,
    cache_ttl: Duration,
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
        copilot_client: CopilotClient::with_api_url(client.clone(), "http://localhost:1/unused".into())
            .with_models_url(models_api_url),
        http_client: client,
        config: AdapterConfig {
            static_models: false,
            ..AdapterConfig::default()
        },
        models_cache: ModelsCache::new(cache_ttl),
    })
}

/// Build AppState with `--static-models` enabled (no API calls).
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

use axum::response::IntoResponse;

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

/// E3-T11: Cache hit returns cached data without making an API call.
#[tokio::test]
async fn cache_hit_returns_cached_data() {
    let (github_addr, _gh) = spawn_mock_github().await;
    let (models_addr, _mh) = spawn_mock_models_api().await;

    let state = create_dynamic_state(
        format!("http://{models_addr}/models"),
        github_addr,
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

/// E3-T12: Cache miss fetches models from the mock Copilot API.
#[tokio::test]
async fn cache_miss_fetches_from_api() {
    let (github_addr, _gh) = spawn_mock_github().await;
    let (models_addr, _mh) = spawn_mock_models_api().await;

    let state = create_dynamic_state(
        format!("http://{models_addr}/models"),
        github_addr,
        Duration::from_secs(300),
    )
    .await;

    // Cache is empty — should trigger API fetch.
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

    // Should contain the dynamic models from our mock API.
    let ids: Vec<&str> = list.data.iter().map(|m| m.id.as_str()).collect();
    assert!(ids.contains(&"gpt-4o-dynamic"), "Should contain gpt-4o-dynamic from mock API");
    assert!(ids.contains(&"claude-sonnet-4"), "Should contain claude-sonnet-4 from mock API");
}

/// E3-T13: API error triggers fallback to the static model list.
#[tokio::test]
async fn api_error_triggers_fallback() {
    let (github_addr, _gh) = spawn_mock_github().await;
    let (failing_addr, _fh) = spawn_failing_models_api().await;

    let state = create_dynamic_state(
        format!("http://{failing_addr}/models"),
        github_addr,
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

    // Should contain fallback models.
    let ids: Vec<&str> = list.data.iter().map(|m| m.id.as_str()).collect();
    assert!(ids.contains(&"gpt-4o"), "Fallback should contain gpt-4o");
    assert!(ids.contains(&"gpt-4"), "Fallback should contain gpt-4");
    assert!(ids.contains(&"gpt-3.5-turbo"), "Fallback should contain gpt-3.5-turbo");
    // Should NOT contain retired models.
    assert!(!ids.contains(&"claude-3.5-sonnet"), "Fallback should not contain retired claude-3.5-sonnet");
}

/// E3-T14: Static models mode always uses fallback without API calls.
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

    // Should contain fallback models.
    let ids: Vec<&str> = list.data.iter().map(|m| m.id.as_str()).collect();
    assert!(ids.contains(&"gpt-4o"), "Should contain gpt-4o");
    assert!(ids.contains(&"gpt-4"), "Should contain gpt-4");
    assert!(ids.contains(&"gpt-4-turbo"), "Should contain gpt-4-turbo");
    assert!(ids.contains(&"gpt-3.5-turbo"), "Should contain gpt-3.5-turbo");
    assert_eq!(list.data.len(), 4, "Fallback list should have exactly 4 models");
}

/// E3-T15: get_model with a valid model ID succeeds.
#[tokio::test]
async fn get_model_valid_id_succeeds() {
    let (github_addr, _gh) = spawn_mock_github().await;
    let (models_addr, _mh) = spawn_mock_models_api().await;

    let state = create_dynamic_state(
        format!("http://{models_addr}/models"),
        github_addr,
        Duration::from_secs(300),
    )
    .await;

    let app = build_router(state);
    let response = app
        .oneshot(
            Request::builder()
                .uri("/v1/models/gpt-4o-dynamic")
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
    assert_eq!(model.id, "gpt-4o-dynamic");
    assert_eq!(model.object, "model");
}

/// E3-T16: get_model with an invalid model ID returns 404.
#[tokio::test]
async fn get_model_invalid_id_returns_404() {
    let (github_addr, _gh) = spawn_mock_github().await;
    let (models_addr, _mh) = spawn_mock_models_api().await;

    let state = create_dynamic_state(
        format!("http://{models_addr}/models"),
        github_addr,
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
