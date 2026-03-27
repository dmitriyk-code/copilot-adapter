//! Reusable mock GitHub OAuth server for integration tests.
//!
//! Provides a mock implementation of the GitHub device flow OAuth endpoints
//! and the Copilot token exchange endpoint, backed by axum.

use std::net::SocketAddr;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;

use axum::http::{HeaderMap, StatusCode};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde_json::json;
use tokio::net::TcpListener;
use tokio::task::JoinHandle;

/// A running mock GitHub server with all standard OAuth endpoints.
pub struct MockGitHub {
    pub addr: SocketAddr,
    pub handle: JoinHandle<()>,
}

impl MockGitHub {
    /// Spawn a mock GitHub server that handles:
    /// - `POST /login/device/code` — device code initiation
    /// - `POST /login/oauth/access_token` — token polling
    /// - `GET  /copilot_internal/v2/token` — Copilot token exchange
    pub async fn spawn() -> Self {
        let app = Router::new()
            .route("/login/device/code", post(mock_device_code))
            .route("/login/oauth/access_token", post(mock_access_token))
            .route("/copilot_internal/v2/token", get(mock_copilot_token));

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let handle = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });

        Self { addr, handle }
    }

    /// Spawn a mock GitHub server that only serves the Copilot token endpoint.
    /// Useful for tests that only need token exchange (e.g., chat tests).
    pub async fn spawn_copilot_token_only() -> Self {
        let app = Router::new().route(
            "/copilot_internal/v2/token",
            get(mock_copilot_token),
        );

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let handle = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });

        Self { addr, handle }
    }

    /// Spawn a mock GitHub server with a request counter on the Copilot token
    /// endpoint. Useful for verifying caching behavior.
    pub async fn spawn_with_counter() -> (Self, Arc<AtomicU32>) {
        let counter = Arc::new(AtomicU32::new(0));
        let counter_clone = counter.clone();

        let app = Router::new().route(
            "/copilot_internal/v2/token",
            get(move |headers: HeaderMap| {
                let c = counter_clone.clone();
                async move {
                    c.fetch_add(1, Ordering::SeqCst);
                    copilot_token_response(&headers)
                }
            }),
        );

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let handle = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });

        (Self { addr, handle }, counter)
    }

    /// URL for the Copilot token endpoint on this mock server.
    pub fn copilot_token_url(&self) -> String {
        format!("http://{}/copilot_internal/v2/token", self.addr)
    }

    /// URL for the device code initiation endpoint.
    pub fn device_code_url(&self) -> String {
        format!("http://{}/login/device/code", self.addr)
    }

    /// URL for the token polling endpoint.
    pub fn token_url(&self) -> String {
        format!("http://{}/login/oauth/access_token", self.addr)
    }
}

/// The standard GitHub token accepted by all mock endpoints.
pub const MOCK_GITHUB_TOKEN: &str = "gho_mock_github_token_xyz";

/// The Copilot token returned by the mock server.
pub const MOCK_COPILOT_TOKEN: &str = "tid_copilot_token_abc";

/// The device code returned by the mock server.
pub const MOCK_DEVICE_CODE: &str = "test_device_code_123";

/// The user code returned by the mock server.
pub const MOCK_USER_CODE: &str = "ABCD-1234";

async fn mock_device_code() -> Json<serde_json::Value> {
    Json(json!({
        "device_code": MOCK_DEVICE_CODE,
        "user_code": MOCK_USER_CODE,
        "verification_uri": "https://github.com/login/device",
        "expires_in": 900,
        "interval": 1
    }))
}

async fn mock_access_token(
    body: axum::extract::Form<Vec<(String, String)>>,
) -> Json<serde_json::Value> {
    let has_device_code = body
        .0
        .iter()
        .any(|(k, v)| k == "device_code" && v == MOCK_DEVICE_CODE);

    if has_device_code {
        Json(json!({
            "access_token": MOCK_GITHUB_TOKEN,
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
    headers: HeaderMap,
) -> Result<Json<serde_json::Value>, StatusCode> {
    Ok(copilot_token_response(&headers))
}

fn copilot_token_response(headers: &HeaderMap) -> Json<serde_json::Value> {
    let auth = headers
        .get("Authorization")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");

    if auth == format!("token {MOCK_GITHUB_TOKEN}")
        || auth == "token test_github_token"
    {
        let expires_at = chrono::Utc::now().timestamp() + 1800;
        Json(json!({
            "token": MOCK_COPILOT_TOKEN,
            "expires_at": expires_at
        }))
    } else {
        Json(json!({"error": "unauthorized"}))
    }
}
