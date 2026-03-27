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
