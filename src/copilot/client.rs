use crate::copilot::types::{ChatCompletionRequest, ChatCompletionResponse};
use crate::error::AppError;

const COPILOT_CHAT_COMPLETIONS_URL: &str =
    "https://api.githubcopilot.com/chat/completions";

// Identity header values sent to the Copilot API.
// Extracted as constants so they are easy to find and update when versions change.
const EDITOR_VERSION: &str = "vscode/1.85.0";
const EDITOR_PLUGIN_VERSION: &str = "copilot-chat/0.12.0";
const COPILOT_INTEGRATION_ID: &str = "vscode-chat";

/// Client for communicating with the GitHub Copilot Chat API.
pub struct CopilotClient {
    /// The underlying HTTP client used for requests.
    /// Reserved as a private field — callers obtain a `CopilotClient` via
    /// `new()` or `with_api_url()` and interact through `send_chat_completion()`.
    client: reqwest::Client,
    api_url: String,
}

impl CopilotClient {
    /// Create a new `CopilotClient` using the default Copilot API URL.
    pub fn new(client: reqwest::Client) -> Self {
        Self {
            client,
            api_url: COPILOT_CHAT_COMPLETIONS_URL.to_string(),
        }
    }

    /// Create a `CopilotClient` with a custom API URL (for testing).
    pub fn with_api_url(client: reqwest::Client, api_url: String) -> Self {
        Self { client, api_url }
    }

    /// Send a non-streaming chat completion request to the Copilot API.
    pub async fn send_chat_completion(
        &self,
        token: &str,
        request: &ChatCompletionRequest,
    ) -> Result<ChatCompletionResponse, AppError> {
        let request_id = uuid::Uuid::new_v4().to_string();

        let response = self
            .client
            .post(&self.api_url)
            .bearer_auth(token)
            .header("Content-Type", "application/json")
            .header("X-Request-Id", &request_id)
            .header("Copilot-Integration-Id", COPILOT_INTEGRATION_ID)
            .header("Editor-Version", EDITOR_VERSION)
            .header("Editor-Plugin-Version", EDITOR_PLUGIN_VERSION)
            .header("Openai-Organization", "github-copilot")
            .header("Openai-Intent", "conversation-agent")
            .json(request)
            .send()
            .await
            .map_err(|e| {
                AppError::Internal(format!("Failed to reach Copilot API: {e}"))
            })?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(AppError::UpstreamError(format!(
                "Copilot API returned HTTP {status}: {body}"
            )));
        }

        response.json::<ChatCompletionResponse>().await.map_err(|e| {
            AppError::Internal(format!("Failed to parse Copilot API response: {e}"))
        })
    }

}
