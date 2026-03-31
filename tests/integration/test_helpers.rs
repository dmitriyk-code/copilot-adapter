use std::sync::Arc;

use copilot_adapter::auth::device_flow::DeviceFlowAuth;
use copilot_adapter::auth::token::TokenManager;
use copilot_adapter::copilot::client::CopilotClient;
use copilot_adapter::copilot::models_cache::ModelsCache;
use copilot_adapter::server::{AdapterConfig, AppState};
use copilot_adapter::storage::TokenStorage;

/// In-memory token storage for tests.
pub struct InMemoryStorage {
    token: std::sync::Mutex<Option<String>>,
}

impl InMemoryStorage {
    pub fn new() -> Self {
        Self {
            token: std::sync::Mutex::new(None),
        }
    }

    pub fn with_token(token: &str) -> Self {
        Self {
            token: std::sync::Mutex::new(Some(token.to_string())),
        }
    }
}

impl TokenStorage for InMemoryStorage {
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

/// Create an `AppState` wired to the given mock servers.
///
/// Shared by tool-related integration test files to avoid duplication.
pub async fn create_test_state(
    copilot_api_url: String,
    github_addr: std::net::SocketAddr,
) -> Arc<AppState> {
    create_test_state_with_config(copilot_api_url, github_addr, AdapterConfig::default()).await
}

/// Create an `AppState` with native tools enabled.
pub async fn create_test_state_native_tools(
    copilot_api_url: String,
    github_addr: std::net::SocketAddr,
) -> Arc<AppState> {
    let config = AdapterConfig {
        native_tools: true,
        ..AdapterConfig::default()
    };
    create_test_state_with_config(copilot_api_url, github_addr, config).await
}

/// Create an `AppState` with a custom `AdapterConfig`.
pub async fn create_test_state_with_config(
    copilot_api_url: String,
    github_addr: std::net::SocketAddr,
    config: AdapterConfig,
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
        copilot_client: CopilotClient::with_api_url(client, copilot_api_url),
        config,
        models_cache: ModelsCache::new(std::time::Duration::from_secs(300)),
        conversation_logger: None,
    })
}
