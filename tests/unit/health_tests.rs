use axum::body::Body;
use axum::http::{Request, StatusCode};
use tower::ServiceExt;

use copilot_adapter::handlers::health::{health, root};
use copilot_adapter::server::build_router;

// --- Direct handler tests ---

#[tokio::test]
async fn root_handler_returns_status_ok_json() {
    let axum::Json(value) = root().await;
    assert_eq!(value, serde_json::json!({"status": "ok"}));
}

#[tokio::test]
async fn health_handler_returns_status_ok_json() {
    let axum::Json(value) = health().await;
    assert_eq!(value, serde_json::json!({"status": "ok"}));
}

// --- Router-level tests using integration helpers ---

mod router_tests {
    use super::*;
    use std::sync::Arc;

    use copilot_adapter::auth::device_flow::DeviceFlowAuth;
    use copilot_adapter::auth::token::TokenManager;
    use copilot_adapter::copilot::client::CopilotClient;
    use copilot_adapter::copilot::models_cache::ModelsCache;
    use copilot_adapter::server::{AdapterConfig, AppState};
    use copilot_adapter::storage::TokenStorage;

    struct InMemoryStorage;

    impl TokenStorage for InMemoryStorage {
        fn store_github_token(&self, _token: &str) -> anyhow::Result<()> {
            Ok(())
        }
        fn get_github_token(&self) -> anyhow::Result<String> {
            Ok("fake".into())
        }
        fn delete_github_token(&self) -> anyhow::Result<()> {
            Ok(())
        }
    }

    async fn test_state() -> Arc<AppState> {
        let auth = DeviceFlowAuth::with_urls(
            "http://localhost:1/unused".into(),
            "http://localhost:1/unused".into(),
            "http://localhost:1/unused".into(),
        );
        let tm = Arc::new(
            TokenManager::new(Box::new(InMemoryStorage), auth)
                .await
                .unwrap(),
        );
        Arc::new(AppState {
            token_manager: tm,
            copilot_client: CopilotClient::new(reqwest::Client::new()),
            config: AdapterConfig::default(),
            models_cache: ModelsCache::new(std::time::Duration::from_secs(300)),
            conversation_logger: None,
        })
    }

    #[tokio::test]
    async fn get_root_returns_200_with_json_body() {
        let state = test_state().await;
        let app = build_router(state);

        let response = app
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri("/")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);

        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json, serde_json::json!({"status": "ok"}));
    }

    #[tokio::test]
    async fn head_root_returns_200_with_empty_body() {
        let state = test_state().await;
        let app = build_router(state);

        let response = app
            .oneshot(
                Request::builder()
                    .method("HEAD")
                    .uri("/")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);

        // HEAD responses must have empty body
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        assert!(body.is_empty(), "HEAD response body should be empty");
    }

    #[tokio::test]
    async fn post_root_returns_405_method_not_allowed() {
        let state = test_state().await;
        let app = build_router(state);

        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::METHOD_NOT_ALLOWED);
    }
}
