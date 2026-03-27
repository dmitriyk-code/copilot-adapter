use bytes::Bytes;
use futures::stream::Stream;

use crate::copilot::types::{ChatCompletionChunk, ChatCompletionRequest, ChatCompletionResponse};
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

    /// Send a streaming chat completion request to the Copilot API.
    ///
    /// Returns a `Stream` of parsed `ChatCompletionChunk` items. The stream
    /// terminates when the upstream sends the `[DONE]` marker.
    pub async fn stream_chat_completion(
        &self,
        token: &str,
        request: &ChatCompletionRequest,
    ) -> Result<impl Stream<Item = Result<ChatCompletionChunk, AppError>>, AppError> {
        let request_id = uuid::Uuid::new_v4().to_string();

        // Build a copy of the request with `stream: true` enforced.
        let mut body = serde_json::to_value(request).map_err(|e| {
            AppError::Internal(format!("Failed to serialize request: {e}"))
        })?;
        body["stream"] = serde_json::Value::Bool(true);

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
            .json(&body)
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

        let byte_stream = response.bytes_stream();
        Ok(parse_sse_stream(byte_stream))
    }

}

/// Parse a raw byte stream of SSE data into a stream of `ChatCompletionChunk`s.
///
/// Handles:
/// - Buffering partial lines across byte boundaries
/// - Extracting `data:` lines from SSE frames (delimited by `\n\n`)
/// - The `[DONE]` sentinel that signals end of stream
/// - Ignoring blank lines and comment lines (starting with `:`)
pub fn parse_sse_stream<S>(byte_stream: S) -> impl Stream<Item = Result<ChatCompletionChunk, AppError>>
where
    S: Stream<Item = Result<Bytes, reqwest::Error>> + Unpin + Send + 'static,
{
    async_stream::try_stream! {
        use futures::StreamExt;

        let mut buf = String::new();
        let mut stream = std::pin::pin!(byte_stream);

        while let Some(chunk_result) = stream.next().await {
            let chunk = chunk_result.map_err(|e| {
                AppError::Internal(format!("Stream read error: {e}"))
            })?;
            buf.push_str(&String::from_utf8_lossy(&chunk));

            // Process all complete SSE frames (delimited by double newline).
            while let Some(pos) = buf.find("\n\n") {
                let frame = buf[..pos].to_string();
                buf = buf[pos + 2..].to_string();

                for line in frame.lines() {
                    let line = line.trim();

                    // Skip empty lines and SSE comments
                    if line.is_empty() || line.starts_with(':') {
                        continue;
                    }

                    if let Some(data) = line.strip_prefix("data:") {
                        let data = data.trim();

                        // [DONE] sentinel — end the stream
                        if data == "[DONE]" {
                            return;
                        }

                        let chunk: ChatCompletionChunk =
                            serde_json::from_str(data).map_err(|e| {
                                AppError::Internal(format!(
                                    "Failed to parse SSE chunk JSON: {e}"
                                ))
                            })?;
                        yield chunk;
                    }
                }
            }
        }
    }
}
