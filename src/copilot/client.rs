use std::time::Duration;

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

/// Maximum number of retries for transient errors (5xx, network timeouts).
const MAX_RETRIES: u32 = 3;

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

    /// Build an HTTP request to the Copilot chat completions endpoint.
    fn build_request(
        &self,
        token: &str,
        body: &serde_json::Value,
        request_id: &str,
    ) -> reqwest::RequestBuilder {
        self.client
            .post(&self.api_url)
            .bearer_auth(token)
            .header("Content-Type", "application/json")
            .header("X-Request-Id", request_id)
            .header("Copilot-Integration-Id", COPILOT_INTEGRATION_ID)
            .header("Editor-Version", EDITOR_VERSION)
            .header("Editor-Plugin-Version", EDITOR_PLUGIN_VERSION)
            .header("Openai-Organization", "github-copilot")
            .header("Openai-Intent", "conversation-agent")
            .json(body)
    }

    /// Parse the `Retry-After` header from an HTTP response, defaulting to 60s.
    fn parse_retry_after(response: &reqwest::Response) -> u64 {
        response
            .headers()
            .get("retry-after")
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.parse::<u64>().ok())
            .unwrap_or(60)
    }

    /// Handle a non-success HTTP response from the Copilot API.
    /// Returns `RateLimited` for 429, `CopilotError` for everything else.
    async fn handle_error_response(response: reqwest::Response) -> AppError {
        let status = response.status();

        if status.as_u16() == 429 {
            let retry_after = Self::parse_retry_after(&response);
            tracing::warn!(retry_after_secs = retry_after, "Rate limited by Copilot API");
            return AppError::RateLimited(retry_after);
        }

        let body = response.text().await.unwrap_or_default();
        tracing::error!(
            status = %status,
            body = %body,
            "Copilot API error response"
        );
        AppError::CopilotError(format!("Copilot API returned HTTP {status}: {body}"))
    }

    /// Send a non-streaming chat completion request to the Copilot API.
    ///
    /// Retries transient errors (5xx, network timeouts) up to 3 times with
    /// exponential backoff (1s, 2s, 4s). Rate-limited (429) errors are
    /// returned immediately with the retry-after value.
    pub async fn send_chat_completion(
        &self,
        token: &str,
        request: &ChatCompletionRequest,
    ) -> Result<ChatCompletionResponse, AppError> {
        let request_id = uuid::Uuid::new_v4().to_string();
        let body = serde_json::to_value(request).map_err(|e| {
            AppError::Internal(format!("Failed to serialize request: {e}"))
        })?;

        tracing::debug!(
            request_id = %request_id,
            model = %request.model,
            "Sending chat completion request to Copilot API"
        );

        let mut last_error: Option<AppError> = None;

        for attempt in 0..=MAX_RETRIES {
            if attempt > 0 {
                let delay = Duration::from_secs(1 << (attempt - 1));
                tracing::info!(
                    request_id = %request_id,
                    attempt = attempt,
                    delay_secs = delay.as_secs(),
                    "Retrying Copilot API request"
                );
                tokio::time::sleep(delay).await;
            }

            let result = self.build_request(token, &body, &request_id).send().await;

            match result {
                Ok(response) => {
                    if response.status().is_success() {
                        tracing::debug!(
                            request_id = %request_id,
                            "Copilot API request succeeded"
                        );
                        return response
                            .json::<ChatCompletionResponse>()
                            .await
                            .map_err(|e| {
                                AppError::Internal(format!(
                                    "Failed to parse Copilot API response: {e}"
                                ))
                            });
                    }

                    let status = response.status();

                    // Rate limits are not retried — surface immediately.
                    if status.as_u16() == 429 {
                        return Err(Self::handle_error_response(response).await);
                    }

                    // Retry on 5xx server errors.
                    if status.is_server_error() && attempt < MAX_RETRIES {
                        let body_text = response.text().await.unwrap_or_default();
                        tracing::warn!(
                            request_id = %request_id,
                            attempt = attempt,
                            status = %status,
                            "Copilot API returned server error, will retry"
                        );
                        last_error = Some(AppError::CopilotError(format!(
                            "Copilot API returned HTTP {status}: {body_text}"
                        )));
                        continue;
                    }

                    return Err(Self::handle_error_response(response).await);
                }
                Err(e) => {
                    let is_transient = e.is_timeout() || e.is_connect();
                    if is_transient && attempt < MAX_RETRIES {
                        tracing::warn!(
                            request_id = %request_id,
                            attempt = attempt,
                            error = %e,
                            "Transient network error, will retry"
                        );
                        last_error = Some(AppError::CopilotError(format!(
                            "Failed to reach Copilot API: {e}"
                        )));
                        continue;
                    }

                    tracing::error!(
                        request_id = %request_id,
                        error = %e,
                        "Failed to reach Copilot API after retries"
                    );
                    return Err(AppError::CopilotError(format!(
                        "Failed to reach Copilot API: {e}"
                    )));
                }
            }
        }

        // Should not be reached, but return the last error if it is.
        Err(last_error.unwrap_or_else(|| {
            AppError::Internal("Unexpected retry loop exit".to_string())
        }))
    }

    /// Send a streaming chat completion request to the Copilot API.
    ///
    /// Returns a `Stream` of parsed `ChatCompletionChunk` items. The stream
    /// terminates when the upstream sends the `[DONE]` marker.
    ///
    /// Retries the initial connection for transient errors (5xx, network
    /// timeouts) up to 3 times with exponential backoff. Once streaming
    /// begins, no further retries are attempted.
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

        tracing::debug!(
            request_id = %request_id,
            model = %request.model,
            "Sending streaming chat completion request to Copilot API"
        );

        let mut last_error: Option<AppError> = None;

        for attempt in 0..=MAX_RETRIES {
            if attempt > 0 {
                let delay = Duration::from_secs(1 << (attempt - 1));
                tracing::info!(
                    request_id = %request_id,
                    attempt = attempt,
                    delay_secs = delay.as_secs(),
                    "Retrying streaming Copilot API request"
                );
                tokio::time::sleep(delay).await;
            }

            let result = self.build_request(token, &body, &request_id)
                .header("Accept", "text/event-stream")
                .send()
                .await;

            match result {
                Ok(response) => {
                    if response.status().is_success() {
                        tracing::debug!(
                            request_id = %request_id,
                            "Streaming Copilot API connection established"
                        );
                        let byte_stream = response.bytes_stream();
                        return Ok(parse_sse_stream(byte_stream));
                    }

                    let status = response.status();

                    if status.as_u16() == 429 {
                        return Err(Self::handle_error_response(response).await);
                    }

                    if status.is_server_error() && attempt < MAX_RETRIES {
                        let body_text = response.text().await.unwrap_or_default();
                        tracing::warn!(
                            request_id = %request_id,
                            attempt = attempt,
                            status = %status,
                            "Streaming: Copilot API returned server error, will retry"
                        );
                        last_error = Some(AppError::CopilotError(format!(
                            "Copilot API returned HTTP {status}: {body_text}"
                        )));
                        continue;
                    }

                    return Err(Self::handle_error_response(response).await);
                }
                Err(e) => {
                    let is_transient = e.is_timeout() || e.is_connect();
                    if is_transient && attempt < MAX_RETRIES {
                        tracing::warn!(
                            request_id = %request_id,
                            attempt = attempt,
                            error = %e,
                            "Streaming: Transient network error, will retry"
                        );
                        last_error = Some(AppError::CopilotError(format!(
                            "Failed to reach Copilot API: {e}"
                        )));
                        continue;
                    }

                    tracing::error!(
                        request_id = %request_id,
                        error = %e,
                        "Failed to reach Copilot API for streaming after retries"
                    );
                    return Err(AppError::CopilotError(format!(
                        "Failed to reach Copilot API: {e}"
                    )));
                }
            }
        }

        Err(last_error.unwrap_or_else(|| {
            AppError::Internal("Unexpected retry loop exit".to_string())
        }))
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
