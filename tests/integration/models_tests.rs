use std::sync::Arc;

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
    })
}

#[tokio::test]
async fn list_models_returns_model_list() {
    let state = test_state().await;
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

    assert_eq!(list.object, "list");
    assert!(!list.data.is_empty());

    // Verify expected models are present
    let model_ids: Vec<&str> = list.data.iter().map(|m| m.id.as_str()).collect();
    assert!(model_ids.contains(&"gpt-4"), "Should contain gpt-4");
    assert!(
        model_ids.contains(&"gpt-3.5-turbo"),
        "Should contain gpt-3.5-turbo"
    );

    // Verify model format
    for model in &list.data {
        assert_eq!(model.object, "model");
        assert_eq!(model.owned_by, "github-copilot");
        assert!(model.created > 0);
    }
}

#[tokio::test]
async fn get_model_returns_specific_model() {
    let state = test_state().await;
    let app = build_router(state);

    let response = app
        .oneshot(
            Request::builder()
                .uri("/v1/models/gpt-4")
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

    assert_eq!(model.id, "gpt-4");
    assert_eq!(model.object, "model");
    assert_eq!(model.owned_by, "github-copilot");
}

#[tokio::test]
async fn get_model_returns_404_for_unknown_model() {
    let state = test_state().await;
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
    assert!(json["error"]["message"]
        .as_str()
        .unwrap()
        .contains("not found"));
}

#[tokio::test]
async fn get_model_gpt4o_returns_model() {
    let state = test_state().await;
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
}

#[tokio::test]
async fn list_models_response_matches_openai_format() {
    let state = test_state().await;
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
    let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();

    // Verify OpenAI list response format
    assert_eq!(json["object"], "list");
    assert!(json["data"].is_array());
    let data = json["data"].as_array().unwrap();
    assert!(!data.is_empty());

    for item in data {
        assert!(item.get("id").is_some());
        assert_eq!(item["object"], "model");
        assert!(item.get("created").is_some());
        assert!(item.get("owned_by").is_some());
    }
}
