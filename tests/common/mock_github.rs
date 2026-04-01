//! Reusable mock GitHub OAuth server for integration tests.
//!
//! Provides a mock implementation of the GitHub device flow OAuth endpoints
//! and the Copilot token exchange endpoint, backed by axum.

use std::net::SocketAddr;

use axum::http::HeaderMap;
use axum::routing::get;
use axum::{Json, Router};
use serde_json::json;
use tokio::net::TcpListener;
use tokio::task::JoinHandle;

/// A running mock GitHub server with Copilot token endpoint.
pub struct MockGitHub {
    pub addr: SocketAddr,
    #[allow(dead_code)] // Kept to prevent server from being dropped
    pub handle: JoinHandle<()>,
}

impl MockGitHub {
    /// Spawn a mock GitHub server that only serves the Copilot token endpoint.
    /// Useful for tests that only need token exchange (e.g., chat tests).
    pub async fn spawn_copilot_token_only() -> Self {
        let app = Router::new().route("/copilot_internal/v2/token", get(mock_copilot_token));

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let handle = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });

        Self { addr, handle }
    }

    /// URL for the Copilot token endpoint on this mock server.
    pub fn copilot_token_url(&self) -> String {
        format!("http://{}/copilot_internal/v2/token", self.addr)
    }
}

/// The standard GitHub token accepted by all mock endpoints.
pub const MOCK_GITHUB_TOKEN: &str = "gho_mock_github_token_xyz";

/// The Copilot token returned by the mock server.
pub const MOCK_COPILOT_TOKEN: &str = "tid_copilot_token_abc";

async fn mock_copilot_token(headers: HeaderMap) -> Json<serde_json::Value> {
    copilot_token_response(&headers)
}

fn copilot_token_response(headers: &HeaderMap) -> Json<serde_json::Value> {
    let auth = headers
        .get("Authorization")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");

    if auth == format!("token {MOCK_GITHUB_TOKEN}") || auth == "token test_github_token" {
        let expires_at = chrono::Utc::now().timestamp() + 1800;
        Json(json!({
            "token": MOCK_COPILOT_TOKEN,
            "expires_at": expires_at
        }))
    } else {
        Json(json!({"error": "unauthorized"}))
    }
}
