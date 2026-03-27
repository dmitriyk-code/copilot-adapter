use anyhow::{anyhow, Result};
use chrono::{DateTime, Utc};
use serde::Deserialize;
use std::time::Duration;

const GITHUB_DEVICE_CODE_URL: &str = "https://github.com/login/device/code";
const GITHUB_TOKEN_URL: &str = "https://github.com/login/oauth/access_token";
const COPILOT_TOKEN_URL: &str = "https://api.github.com/copilot_internal/v2/token";
const CLIENT_ID: &str = "Iv1.b507a08c87ecfe98";

/// Response from GitHub's device code initiation endpoint.
#[derive(Debug, Deserialize)]
pub struct DeviceCodeResponse {
    pub device_code: String,
    pub user_code: String,
    pub verification_uri: String,
    pub expires_in: u64,
    pub interval: u64,
}

/// A short-lived Copilot API token.
#[derive(Debug, Clone, Deserialize)]
pub struct CopilotToken {
    pub token: String,
    pub expires_at: i64,
}

impl CopilotToken {
    /// Returns true if the token has not yet expired.
    pub fn is_valid(&self) -> bool {
        let expires = DateTime::from_timestamp(self.expires_at, 0);
        match expires {
            Some(exp) => Utc::now() < exp,
            None => false,
        }
    }

    /// Returns the expiry time as a `DateTime<Utc>`, or None if the timestamp is invalid.
    pub fn expires_at_datetime(&self) -> Option<DateTime<Utc>> {
        DateTime::from_timestamp(self.expires_at, 0)
    }

    /// Seconds remaining until expiry (0 if already expired).
    pub fn seconds_until_expiry(&self) -> u64 {
        let now = Utc::now().timestamp();
        if self.expires_at > now {
            (self.expires_at - now) as u64
        } else {
            0
        }
    }
}

/// Handles the GitHub OAuth device flow and Copilot token exchange.
pub struct DeviceFlowAuth {
    client: reqwest::Client,
    device_code_url: String,
    token_url: String,
    copilot_token_url: String,
}

impl DeviceFlowAuth {
    pub fn new() -> Self {
        Self {
            client: reqwest::Client::new(),
            device_code_url: GITHUB_DEVICE_CODE_URL.to_string(),
            token_url: GITHUB_TOKEN_URL.to_string(),
            copilot_token_url: COPILOT_TOKEN_URL.to_string(),
        }
    }

    /// Create a DeviceFlowAuth with custom URLs (for testing with mock servers).
    pub fn with_urls(
        device_code_url: String,
        token_url: String,
        copilot_token_url: String,
    ) -> Self {
        Self {
            client: reqwest::Client::new(),
            device_code_url,
            token_url,
            copilot_token_url,
        }
    }

    /// Initiate the device flow: request a device code from GitHub.
    pub async fn initiate(&self) -> Result<DeviceCodeResponse> {
        let response = self
            .client
            .post(&self.device_code_url)
            .header("Accept", "application/json")
            .header("User-Agent", "github4claude")
            .form(&[("client_id", CLIENT_ID), ("scope", "read:user")])
            .send()
            .await?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(anyhow!(
                "Failed to initiate device flow: HTTP {status} — {body}"
            ));
        }

        Ok(response.json().await?)
    }

    /// Poll GitHub for the access token after the user authorises in their browser.
    /// Returns the GitHub OAuth access token on success.
    pub async fn poll_for_token(
        &self,
        device_code: &str,
        interval: u64,
        expires_in: u64,
    ) -> Result<String> {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(expires_in);
        let mut poll_interval = Duration::from_secs(interval);

        loop {
            tokio::time::sleep(poll_interval).await;

            if tokio::time::Instant::now() >= deadline {
                return Err(anyhow!("Device flow authorization timed out"));
            }

            let response = self
                .client
                .post(&self.token_url)
                .header("Accept", "application/json")
                .header("User-Agent", "github4claude")
                .form(&[
                    ("client_id", CLIENT_ID),
                    ("device_code", device_code),
                    (
                        "grant_type",
                        "urn:ietf:params:oauth:grant-type:device_code",
                    ),
                ])
                .send()
                .await?;

            let body: serde_json::Value = response.json().await?;

            // Success — access_token is present
            if let Some(token) = body.get("access_token").and_then(|v| v.as_str()) {
                return Ok(token.to_string());
            }

            // Check error field
            let error = body
                .get("error")
                .and_then(|v| v.as_str())
                .unwrap_or("");

            match error {
                "authorization_pending" => continue,
                "slow_down" => {
                    // GitHub asks us to add 5 seconds to the interval
                    poll_interval += Duration::from_secs(5);
                    continue;
                }
                "expired_token" => {
                    return Err(anyhow!("Device code expired — please restart authentication"));
                }
                "access_denied" => {
                    return Err(anyhow!("Authorization was denied by the user"));
                }
                other => {
                    let desc = body
                        .get("error_description")
                        .and_then(|v| v.as_str())
                        .unwrap_or("Unknown error");
                    return Err(anyhow!("OAuth error: {other} — {desc}"));
                }
            }
        }
    }

    /// Exchange a GitHub access token for a short-lived Copilot API token.
    pub async fn get_copilot_token(&self, github_token: &str) -> Result<CopilotToken> {
        let response = self
            .client
            .get(&self.copilot_token_url)
            .header("Authorization", format!("token {github_token}"))
            .header("Accept", "application/json")
            .header("User-Agent", "github4claude")
            .send()
            .await?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(anyhow!(
                "Failed to get Copilot token: HTTP {status} — {body}"
            ));
        }

        let token: CopilotToken = response.json().await?;
        Ok(token)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn copilot_token_is_valid_when_not_expired() {
        let future_ts = Utc::now().timestamp() + 3600;
        let token = CopilotToken {
            token: "test".into(),
            expires_at: future_ts,
        };
        assert!(token.is_valid());
    }

    #[test]
    fn copilot_token_is_invalid_when_expired() {
        let past_ts = Utc::now().timestamp() - 60;
        let token = CopilotToken {
            token: "test".into(),
            expires_at: past_ts,
        };
        assert!(!token.is_valid());
    }

    #[test]
    fn seconds_until_expiry_returns_correct_value() {
        let future_ts = Utc::now().timestamp() + 300;
        let token = CopilotToken {
            token: "test".into(),
            expires_at: future_ts,
        };
        let secs = token.seconds_until_expiry();
        assert!(secs >= 299 && secs <= 301);
    }

    #[test]
    fn seconds_until_expiry_returns_zero_when_expired() {
        let past_ts = Utc::now().timestamp() - 60;
        let token = CopilotToken {
            token: "test".into(),
            expires_at: past_ts,
        };
        assert_eq!(token.seconds_until_expiry(), 0);
    }
}
