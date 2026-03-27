use axum::http::StatusCode;
use axum::routing::{get, post};
use axum::Json;
use axum::Router;
use serde_json::json;
use std::net::SocketAddr;
use tokio::net::TcpListener;

use copilot_adapter::auth::device_flow::DeviceFlowAuth;

/// Spawn a mock GitHub OAuth server that handles device code and token exchange.
async fn spawn_mock_github_server() -> (SocketAddr, tokio::task::JoinHandle<()>) {
    let app = Router::new()
        .route("/login/device/code", post(mock_device_code))
        .route("/login/oauth/access_token", post(mock_access_token))
        .route("/copilot_internal/v2/token", get(mock_copilot_token));

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    let handle = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });

    (addr, handle)
}

async fn mock_device_code() -> Json<serde_json::Value> {
    Json(json!({
        "device_code": "test_device_code_123",
        "user_code": "ABCD-1234",
        "verification_uri": "https://github.com/login/device",
        "expires_in": 900,
        "interval": 1
    }))
}

async fn mock_access_token(
    body: axum::extract::Form<Vec<(String, String)>>,
) -> Json<serde_json::Value> {
    // Check that device_code is present
    let has_device_code = body
        .0
        .iter()
        .any(|(k, v)| k == "device_code" && v == "test_device_code_123");

    if has_device_code {
        // Immediately grant the token (no pending cycle in tests)
        Json(json!({
            "access_token": "gho_mock_github_token_xyz",
            "token_type": "bearer",
            "scope": "read:user"
        }))
    } else {
        Json(json!({
            "error": "bad_verification_code",
            "error_description": "The device code is invalid"
        }))
    }
}

async fn mock_copilot_token(
    headers: axum::http::HeaderMap,
) -> Result<Json<serde_json::Value>, StatusCode> {
    let auth = headers
        .get("Authorization")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");

    if auth == "token gho_mock_github_token_xyz" {
        let expires_at = chrono::Utc::now().timestamp() + 1800; // 30 min
        Ok(Json(json!({
            "token": "tid_copilot_token_abc",
            "expires_at": expires_at
        })))
    } else {
        Err(StatusCode::UNAUTHORIZED)
    }
}

#[tokio::test]
async fn device_flow_initiate_returns_device_code() {
    let (addr, _handle) = spawn_mock_github_server().await;

    let auth = DeviceFlowAuth::with_urls(
        format!("http://{addr}/login/device/code"),
        format!("http://{addr}/login/oauth/access_token"),
        format!("http://{addr}/copilot_internal/v2/token"),
    );

    let response = auth.initiate().await.unwrap();
    assert_eq!(response.device_code, "test_device_code_123");
    assert_eq!(response.user_code, "ABCD-1234");
    assert_eq!(response.verification_uri, "https://github.com/login/device");
    assert_eq!(response.interval, 1);
}

#[tokio::test]
async fn device_flow_poll_returns_access_token() {
    let (addr, _handle) = spawn_mock_github_server().await;

    let auth = DeviceFlowAuth::with_urls(
        format!("http://{addr}/login/device/code"),
        format!("http://{addr}/login/oauth/access_token"),
        format!("http://{addr}/copilot_internal/v2/token"),
    );

    let token = auth
        .poll_for_token("test_device_code_123", 1, 30)
        .await
        .unwrap();
    assert_eq!(token, "gho_mock_github_token_xyz");
}

#[tokio::test]
async fn get_copilot_token_succeeds_with_valid_github_token() {
    let (addr, _handle) = spawn_mock_github_server().await;

    let auth = DeviceFlowAuth::with_urls(
        format!("http://{addr}/login/device/code"),
        format!("http://{addr}/login/oauth/access_token"),
        format!("http://{addr}/copilot_internal/v2/token"),
    );

    let copilot = auth
        .get_copilot_token("gho_mock_github_token_xyz")
        .await
        .unwrap();
    assert_eq!(copilot.token, "tid_copilot_token_abc");
    assert!(copilot.is_valid());
}

#[tokio::test]
async fn get_copilot_token_fails_with_invalid_github_token() {
    let (addr, _handle) = spawn_mock_github_server().await;

    let auth = DeviceFlowAuth::with_urls(
        format!("http://{addr}/login/device/code"),
        format!("http://{addr}/login/oauth/access_token"),
        format!("http://{addr}/copilot_internal/v2/token"),
    );

    let result = auth.get_copilot_token("invalid_token").await;
    assert!(result.is_err());
}

#[tokio::test]
async fn full_device_flow_end_to_end() {
    let (addr, _handle) = spawn_mock_github_server().await;

    let auth = DeviceFlowAuth::with_urls(
        format!("http://{addr}/login/device/code"),
        format!("http://{addr}/login/oauth/access_token"),
        format!("http://{addr}/copilot_internal/v2/token"),
    );

    // Step 1: Initiate
    let device = auth.initiate().await.unwrap();
    assert!(!device.user_code.is_empty());
    assert!(!device.verification_uri.is_empty());

    // Step 2: Poll for token (mock grants immediately)
    let github_token = auth
        .poll_for_token(&device.device_code, device.interval, device.expires_in)
        .await
        .unwrap();
    assert!(!github_token.is_empty());

    // Step 3: Exchange for Copilot token
    let copilot = auth.get_copilot_token(&github_token).await.unwrap();
    assert!(!copilot.token.is_empty());
    assert!(copilot.is_valid());
}

/// Test with a mock that returns "authorization_pending" before success.
#[tokio::test]
async fn poll_handles_pending_then_success() {
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::sync::Arc;

    let counter = Arc::new(AtomicU32::new(0));
    let counter_clone = counter.clone();

    let app = Router::new()
        .route(
            "/login/oauth/access_token",
            post(move || {
                let c = counter_clone.clone();
                async move {
                    let count = c.fetch_add(1, Ordering::SeqCst);
                    if count < 2 {
                        Json(json!({
                            "error": "authorization_pending",
                            "error_description": "User hasn't authorized yet"
                        }))
                    } else {
                        Json(json!({
                            "access_token": "gho_after_pending",
                            "token_type": "bearer",
                            "scope": "read:user"
                        }))
                    }
                }
            }),
        )
        .route("/login/device/code", post(mock_device_code));

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });

    let auth = DeviceFlowAuth::with_urls(
        format!("http://{addr}/login/device/code"),
        format!("http://{addr}/login/oauth/access_token"),
        format!("http://{addr}/copilot_internal/v2/token"),
    );

    let token = auth.poll_for_token("test_device_code_123", 1, 30).await.unwrap();
    assert_eq!(token, "gho_after_pending");
    assert!(counter.load(Ordering::SeqCst) >= 3);
}
