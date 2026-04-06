use chrono::Utc;
use copilot_adapter::auth::device_flow::CopilotToken;

#[test]
fn token_is_valid_when_future_expiry() {
    let token = CopilotToken {
        token: "test_token".into(),
        expires_at: Utc::now().timestamp() + 1800, // 30 min from now
    };
    assert!(token.is_valid());
}

#[test]
fn token_is_invalid_when_past_expiry() {
    let token = CopilotToken {
        token: "test_token".into(),
        expires_at: Utc::now().timestamp() - 60, // 1 min ago
    };
    assert!(!token.is_valid());
}

#[test]
fn token_is_invalid_at_epoch_zero() {
    let token = CopilotToken {
        token: "test_token".into(),
        expires_at: 0,
    };
    assert!(!token.is_valid());
}

#[test]
fn seconds_until_expiry_positive() {
    let token = CopilotToken {
        token: "test".into(),
        expires_at: Utc::now().timestamp() + 600,
    };
    let secs = token.seconds_until_expiry();
    assert!((599..=601).contains(&secs));
}

#[test]
fn seconds_until_expiry_zero_when_expired() {
    let token = CopilotToken {
        token: "test".into(),
        expires_at: Utc::now().timestamp() - 100,
    };
    assert_eq!(token.seconds_until_expiry(), 0);
}

#[test]
fn expires_at_datetime_valid_timestamp() {
    let ts = Utc::now().timestamp() + 3600;
    let token = CopilotToken {
        token: "test".into(),
        expires_at: ts,
    };
    let dt = token.expires_at_datetime().unwrap();
    assert_eq!(dt.timestamp(), ts);
}

#[test]
fn copilot_token_deserializes_from_json() {
    let json = r#"{"token": "ghu_abc123", "expires_at": 1700000000}"#;
    let token: CopilotToken = serde_json::from_str(json).unwrap();
    assert_eq!(token.token, "ghu_abc123");
    assert_eq!(token.expires_at, 1700000000);
}

// ── TokenManager tests ───────────────────────────────────────────────────────

use copilot_adapter::auth::device_flow::DeviceFlowAuth;
use copilot_adapter::auth::token::TokenManager;
use copilot_adapter::storage::TokenStorage;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;

/// In-memory token storage for tests.
struct MemoryStorage {
    token: std::sync::Mutex<Option<String>>,
}

impl MemoryStorage {
    fn new(token: Option<String>) -> Self {
        Self {
            token: std::sync::Mutex::new(token),
        }
    }
}

impl TokenStorage for MemoryStorage {
    fn store_github_token(&self, token: &str) -> anyhow::Result<()> {
        *self.token.lock().unwrap() = Some(token.to_string());
        Ok(())
    }

    fn get_github_token(&self) -> anyhow::Result<String> {
        self.token
            .lock()
            .unwrap()
            .clone()
            .ok_or_else(|| anyhow::anyhow!("No token"))
    }

    fn delete_github_token(&self) -> anyhow::Result<()> {
        *self.token.lock().unwrap() = None;
        Ok(())
    }
}

/// Spawn a mock Copilot token server that counts requests.
async fn spawn_copilot_mock() -> (
    std::net::SocketAddr,
    Arc<AtomicU32>,
    tokio::task::JoinHandle<()>,
) {
    use axum::routing::get;
    use axum::Json;
    use axum::Router;
    use serde_json::json;
    use tokio::net::TcpListener;

    let counter = Arc::new(AtomicU32::new(0));
    let counter_clone = counter.clone();

    let app = Router::new().route(
        "/copilot_internal/v2/token",
        get(move |headers: axum::http::HeaderMap| {
            let c = counter_clone.clone();
            async move {
                c.fetch_add(1, Ordering::SeqCst);
                let auth = headers
                    .get("Authorization")
                    .and_then(|v| v.to_str().ok())
                    .unwrap_or("");
                if auth == "token ghp_test_token" {
                    let expires_at = chrono::Utc::now().timestamp() + 1800;
                    Json(json!({
                        "token": "tid_copilot_abc",
                        "expires_at": expires_at
                    }))
                } else {
                    Json(json!({
                        "error": "unauthorized"
                    }))
                }
            }
        }),
    );

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let handle = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });

    (addr, counter, handle)
}

fn make_auth(addr: std::net::SocketAddr) -> DeviceFlowAuth {
    DeviceFlowAuth::with_urls(
        format!("http://{addr}/login/device/code"),
        format!("http://{addr}/login/oauth/access_token"),
        format!("http://{addr}/copilot_internal/v2/token"),
    )
}

#[tokio::test]
async fn get_valid_token_fast_path_skips_refresh() {
    let (addr, counter, _handle) = spawn_copilot_mock().await;
    let storage = MemoryStorage::new(Some("ghp_test_token".into()));
    let manager = TokenManager::new(Box::new(storage), make_auth(addr))
        .await
        .unwrap();

    // First call: populates copilot token (1 HTTP request)
    let token1 = manager.get_valid_token().await.unwrap();
    assert_eq!(token1, "tid_copilot_abc");
    assert_eq!(counter.load(Ordering::SeqCst), 1);

    // Second call: token is still valid — fast path, no HTTP request
    let token2 = manager.get_valid_token().await.unwrap();
    assert_eq!(token2, "tid_copilot_abc");
    assert_eq!(
        counter.load(Ordering::SeqCst),
        1,
        "fast path should not call the server again"
    );
}

#[tokio::test]
async fn get_valid_token_errors_when_not_authenticated() {
    let (addr, _counter, _handle) = spawn_copilot_mock().await;
    let storage = MemoryStorage::new(None);
    let manager = TokenManager::new(Box::new(storage), make_auth(addr))
        .await
        .unwrap();

    let result = manager.get_valid_token().await;
    assert!(result.is_err());
    let msg = result.unwrap_err().to_string();
    assert!(
        msg.contains("Not authenticated"),
        "Expected 'Not authenticated' error, got: {msg}"
    );
}

#[tokio::test]
async fn clear_tokens_cancels_auto_refresh() {
    let (addr, _counter, _handle) = spawn_copilot_mock().await;
    let storage = MemoryStorage::new(Some("ghp_test_token".into()));
    let manager = Arc::new(
        TokenManager::new(Box::new(storage), make_auth(addr))
            .await
            .unwrap(),
    );

    // Populate copilot token so auto-refresh has something to work with
    manager.get_valid_token().await.unwrap();

    // Start auto-refresh
    let handle = Arc::clone(&manager).start_auto_refresh();

    // Clear tokens — should cancel the background task
    manager.clear_tokens().await.unwrap();

    // The JoinHandle should complete within a short time
    let result = tokio::time::timeout(std::time::Duration::from_secs(5), handle).await;
    assert!(
        result.is_ok(),
        "auto-refresh task should have been cancelled"
    );

    // Verify state is cleared
    assert!(!manager.is_authenticated().await);
}

#[tokio::test]
async fn stop_auto_refresh_cancels_task() {
    let (addr, _counter, _handle) = spawn_copilot_mock().await;
    let storage = MemoryStorage::new(Some("ghp_test_token".into()));
    let manager = Arc::new(
        TokenManager::new(Box::new(storage), make_auth(addr))
            .await
            .unwrap(),
    );

    // Populate copilot token so auto-refresh has something to work with
    manager.get_valid_token().await.unwrap();

    // Start auto-refresh
    let handle = Arc::clone(&manager).start_auto_refresh();

    // stop_auto_refresh() should cancel the background task without clearing tokens
    manager.stop_auto_refresh();

    // The JoinHandle should complete within a short time
    let result = tokio::time::timeout(std::time::Duration::from_secs(5), handle).await;
    assert!(
        result.is_ok(),
        "auto-refresh task should have been cancelled by stop_auto_refresh()"
    );

    // Unlike clear_tokens(), the manager should still be authenticated
    assert!(manager.is_authenticated().await);
}

/// Spawn a mock Copilot token server that returns short-lived tokens and counts requests.
async fn spawn_short_lived_copilot_mock() -> (
    std::net::SocketAddr,
    Arc<AtomicU32>,
    tokio::task::JoinHandle<()>,
) {
    use axum::routing::get;
    use axum::Json;
    use axum::Router;
    use serde_json::json;
    use tokio::net::TcpListener;

    let counter = Arc::new(AtomicU32::new(0));
    let counter_clone = counter.clone();

    let app = Router::new().route(
        "/copilot_internal/v2/token",
        get(move |headers: axum::http::HeaderMap| {
            let c = counter_clone.clone();
            async move {
                c.fetch_add(1, Ordering::SeqCst);
                let auth = headers
                    .get("Authorization")
                    .and_then(|v| v.to_str().ok())
                    .unwrap_or("");
                if auth == "token ghp_test_token" {
                    // Token expires in 10 seconds — within the 5-minute window,
                    // so the auto-refresh task will sleep only 5 seconds before refreshing.
                    let expires_at = chrono::Utc::now().timestamp() + 10;
                    Json(json!({
                        "token": "tid_short_lived",
                        "expires_at": expires_at
                    }))
                } else {
                    Json(json!({
                        "error": "unauthorized"
                    }))
                }
            }
        }),
    );

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let handle = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });

    (addr, counter, handle)
}

#[tokio::test]
async fn auto_refresh_task_refreshes_before_expiry() {
    let (addr, counter, _handle) = spawn_short_lived_copilot_mock().await;
    let storage = MemoryStorage::new(Some("ghp_test_token".into()));
    let manager = Arc::new(
        TokenManager::new(Box::new(storage), make_auth(addr))
            .await
            .unwrap(),
    );

    // Populate copilot token (1 HTTP request) — returns a 10-second token
    manager.get_valid_token().await.unwrap();
    assert_eq!(counter.load(Ordering::SeqCst), 1);

    // Start auto-refresh — token has <=300 seconds left, so task sleeps 5s then refreshes
    let handle = Arc::clone(&manager).start_auto_refresh();

    // Wait 7 seconds — the auto-refresh should have fired after ~5 seconds
    tokio::time::sleep(std::time::Duration::from_secs(7)).await;

    // Verify the auto-refresh triggered a proactive refresh
    let refresh_count = counter.load(Ordering::SeqCst);
    assert!(
        refresh_count >= 2,
        "Expected auto-refresh to trigger at least 1 proactive refresh, but total API calls = {refresh_count}"
    );

    // Clean up
    manager.stop_auto_refresh();
    let result = tokio::time::timeout(std::time::Duration::from_secs(5), handle).await;
    assert!(result.is_ok(), "auto-refresh task should have been cancelled");
}

#[tokio::test]
async fn auto_refresh_no_panic_without_token() {
    let (addr, _counter, _handle) = spawn_copilot_mock().await;
    // No GitHub token — simulates first startup before authentication
    let storage = MemoryStorage::new(None);
    let manager = Arc::new(
        TokenManager::new(Box::new(storage), make_auth(addr))
            .await
            .unwrap(),
    );

    // Start auto-refresh without any token — should not panic
    let handle = Arc::clone(&manager).start_auto_refresh();

    // Let the task run briefly (it should be waiting 60s, not panicking)
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;

    // Cancel the task
    manager.stop_auto_refresh();

    // The JoinHandle should complete within a short time
    let result = tokio::time::timeout(std::time::Duration::from_secs(5), handle).await;
    assert!(
        result.is_ok(),
        "auto-refresh task should have been cancelled"
    );
}
