use std::sync::Arc;
use std::time::Duration;

use anyhow::{anyhow, Result};
use tokio::sync::{Mutex, RwLock};
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

use crate::auth::device_flow::{CopilotToken, DeviceFlowAuth};
use crate::storage::TokenStorage;

/// Shared token state managed by `TokenManager`.
struct TokenState {
    github_token: Option<String>,
    copilot_token: Option<CopilotToken>,
}

/// Manages GitHub and Copilot tokens with thread-safe access and auto-refresh.
pub struct TokenManager {
    state: RwLock<TokenState>,
    /// Mutex to serialize refresh operations (prevents TOCTOU race).
    refresh_lock: Mutex<()>,
    auth_client: DeviceFlowAuth,
    storage: Box<dyn TokenStorage + Send + Sync>,
    /// Cancellation token for the auto-refresh background task.
    /// Wrapped in std::sync::Mutex so it can be replaced after cancellation
    /// (CancellationToken cannot be un-cancelled once triggered).
    cancel: std::sync::Mutex<CancellationToken>,
}

impl TokenManager {
    /// Create a new `TokenManager`. Attempts to load the GitHub token from storage.
    pub async fn new(
        storage: Box<dyn TokenStorage + Send + Sync>,
        auth_client: DeviceFlowAuth,
    ) -> Result<Self> {
        let github_token = match storage.get_github_token() {
            Ok(token) => {
                tracing::info!("Loaded GitHub token from storage");
                Some(token)
            }
            Err(e) => {
                tracing::debug!(error = %e, "No GitHub token in storage (not authenticated yet)");
                None
            }
        };

        Ok(Self {
            state: RwLock::new(TokenState {
                github_token,
                copilot_token: None,
            }),
            refresh_lock: Mutex::new(()),
            auth_client,
            storage,
            cancel: std::sync::Mutex::new(CancellationToken::new()),
        })
    }

    /// Returns true if a GitHub token is stored (user has authenticated).
    pub async fn is_authenticated(&self) -> bool {
        self.state.read().await.github_token.is_some()
    }

    /// Store a new GitHub access token (after successful device flow auth).
    pub async fn set_github_token(&self, token: String) -> Result<()> {
        self.storage.store_github_token(&token)?;
        self.state.write().await.github_token = Some(token);
        Ok(())
    }

    /// Get a valid Copilot token, refreshing if necessary.
    /// Uses a refresh lock to prevent concurrent redundant refreshes (TOCTOU).
    pub async fn get_valid_token(&self) -> Result<String> {
        // Fast path: check if current copilot token is still valid
        {
            let state = self.state.read().await;
            if let Some(ref ct) = state.copilot_token {
                if ct.is_valid() {
                    return Ok(ct.token.clone());
                }
            }
        }

        // Acquire refresh lock to prevent concurrent refreshes
        let _guard = self.refresh_lock.lock().await;

        // Re-check after acquiring lock — another caller may have refreshed
        {
            let state = self.state.read().await;
            if let Some(ref ct) = state.copilot_token {
                if ct.is_valid() {
                    return Ok(ct.token.clone());
                }
            }
        }

        self.do_refresh().await
    }

    /// Refresh the Copilot token by exchanging the stored GitHub token.
    /// Acquires the refresh lock to prevent concurrent API calls.
    /// Prefer `get_valid_token()` for normal usage — it skips the refresh
    /// if the current token is still valid.
    pub(crate) async fn refresh_copilot_token(&self) -> Result<String> {
        let _guard = self.refresh_lock.lock().await;
        self.do_refresh().await
    }

    /// Inner refresh logic — caller must hold `refresh_lock`.
    async fn do_refresh(&self) -> Result<String> {
        let github_token = {
            let state = self.state.read().await;
            state
                .github_token
                .clone()
                .ok_or_else(|| anyhow!("Not authenticated — run `copilot-adapter auth` first"))?
        };

        tracing::info!("Refreshing Copilot token");

        let new_token = self.auth_client.get_copilot_token(&github_token).await?;
        let token_string = new_token.token.clone();

        if let Some(exp) = new_token.expires_at_datetime() {
            tracing::info!(
                expires_at = %exp,
                seconds_remaining = new_token.seconds_until_expiry(),
                "Copilot token refreshed successfully"
            );
        } else {
            tracing::info!("Copilot token refreshed (unknown expiry)");
        }

        self.state.write().await.copilot_token = Some(new_token);

        Ok(token_string)
    }

    /// Spawn a background task that refreshes the Copilot token 5 minutes before expiry.
    /// The task respects the internal cancellation token and will stop on `clear_tokens()`.
    ///
    /// **Note:** This method is not yet called in production code paths.
    /// Server-mode integration (calling `start_auto_refresh()` from `server::run` and
    /// cancelling on shutdown) is deferred to Epic 5.
    pub fn start_auto_refresh(self: Arc<Self>) -> JoinHandle<()> {
        let cancel = self.cancel.lock().unwrap_or_else(|e| e.into_inner()).clone();
        tokio::spawn(async move {
            loop {
                let sleep_duration = {
                    let state = self.state.read().await;
                    if let Some(ref ct) = state.copilot_token {
                        let secs = ct.seconds_until_expiry();
                        if secs > 300 {
                            // Refresh 5 minutes before expiry
                            Duration::from_secs(secs - 300)
                        } else {
                            // Token is about to expire or already expired — refresh soon
                            Duration::from_secs(5)
                        }
                    } else {
                        // No token yet — wait before checking again
                        Duration::from_secs(60)
                    }
                };

                // Wait for either the sleep duration or cancellation
                tokio::select! {
                    _ = tokio::time::sleep(sleep_duration) => {}
                    _ = cancel.cancelled() => {
                        tracing::debug!("Auto-refresh task cancelled");
                        return;
                    }
                }

                // Check cancellation again before refreshing
                if cancel.is_cancelled() {
                    return;
                }

                // Refresh through the lock to avoid racing with get_valid_token()
                match self.refresh_copilot_token().await {
                    Ok(_) => {
                        tracing::debug!("Copilot token auto-refreshed successfully");
                    }
                    Err(e) => {
                        // If github_token was cleared (logout) between the sleep and
                        // the refresh call, the error is expected — log at debug, not error.
                        if self.state.read().await.github_token.is_none() {
                            tracing::debug!("Auto-refresh skipped: not authenticated");
                        } else {
                            tracing::error!("Failed to auto-refresh Copilot token: {e}");
                        }
                    }
                }
            }
        })
    }

    /// Clear all tokens from memory and storage (for logout).
    /// Also cancels the auto-refresh background task if running, and replaces
    /// the cancellation token so future calls to `start_auto_refresh()` work
    /// correctly (e.g., after re-authentication in server mode).
    pub async fn clear_tokens(&self) -> Result<()> {
        tracing::info!("Clearing all tokens (logout)");
        {
            let mut cancel = self.cancel.lock().unwrap_or_else(|e| e.into_inner());
            cancel.cancel();
            *cancel = CancellationToken::new();
        }
        let mut state = self.state.write().await;
        state.github_token = None;
        state.copilot_token = None;
        self.storage.delete_github_token()?;
        tracing::info!("All tokens cleared");
        Ok(())
    }

    /// Get a reference to the inner `DeviceFlowAuth` for initiating auth flows.
    pub fn auth_client(&self) -> &DeviceFlowAuth {
        &self.auth_client
    }
}
