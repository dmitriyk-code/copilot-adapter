use serde::{Deserialize, Serialize};

use crate::tools::types::ToolCall;

/// Image URL reference in a content block (OpenAI multimodal format).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImageUrl {
    pub url: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

/// Content block in a message (used by Claude models and OpenAI multimodal).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ContentBlock {
    Text { text: String },
    ImageUrl { image_url: ImageUrl },
    #[serde(other)]
    Other,
}

/// Message content can be a string, an array of content blocks, or null.
///
/// Native tool call responses from the OpenAI API typically use `"content": null`.
/// The custom deserializer handles this by treating `null` as empty text.
#[derive(Debug, Clone, Serialize)]
#[serde(untagged)]
pub enum MessageContent {
    Text(String),
    Blocks(Vec<ContentBlock>),
}

impl<'de> serde::Deserialize<'de> for MessageContent {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        use serde::de;

        let value: Option<serde_json::Value> = Option::deserialize(deserializer)?;
        match value {
            None => Ok(MessageContent::Text(String::new())),
            Some(serde_json::Value::String(s)) => Ok(MessageContent::Text(s)),
            Some(serde_json::Value::Array(arr)) => {
                let blocks: Vec<ContentBlock> =
                    serde_json::from_value(serde_json::Value::Array(arr))
                        .map_err(de::Error::custom)?;
                Ok(MessageContent::Blocks(blocks))
            }
            Some(other) => Err(de::Error::custom(format!(
                "expected string, array, or null for message content, got: {}",
                other
            ))),
        }
    }
}

impl Default for MessageContent {
    fn default() -> Self {
        MessageContent::Text(String::new())
    }
}

impl MessageContent {
    /// Extract the text content, joining multiple blocks if necessary.
    pub fn as_text(&self) -> String {
        match self {
            MessageContent::Text(s) => s.clone(),
            MessageContent::Blocks(blocks) => blocks
                .iter()
                .filter_map(|b| match b {
                    ContentBlock::Text { text } => Some(text.as_str()),
                    ContentBlock::ImageUrl { .. } => None,
                    ContentBlock::Other => None,
                })
                .collect::<Vec<_>>()
                .join(""),
        }
    }
}

/// A chat message in OpenAI format.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    pub role: String,
    #[serde(default)]
    pub content: MessageContent,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<Vec<ToolCall>>,
    /// Tool call ID for messages with role "tool".
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
}

/// OpenAI-compatible chat completion request.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatCompletionRequest {
    pub model: String,
    pub messages: Vec<Message>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stream: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_tokens: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub top_p: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub n: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stop: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub presence_penalty: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub frequency_penalty: Option<f64>,
    /// Native OpenAI tool definitions.
    ///
    /// When using native function calling, these are forwarded to the Copilot API.
    /// When using prompt injection (legacy path), these are consumed by the
    /// injector and then set to `None` before sending the request.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tools: Option<Vec<OpenAITool>>,
    /// Tool choice preference ("auto", "none", "required", or specific tool).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_choice: Option<serde_json::Value>,
}

/// A single choice in a chat completion response.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Choice {
    /// Index of this choice. Optional because Claude models via Copilot API omit it.
    #[serde(default)]
    pub index: u32,
    pub message: Message,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub finish_reason: Option<String>,
}

/// Token usage statistics.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Usage {
    #[serde(default)]
    pub prompt_tokens: u32,
    #[serde(default)]
    pub completion_tokens: u32,
    #[serde(default)]
    pub total_tokens: u32,
    /// Additional token details (e.g., cached_tokens). Ignored but accepted.
    #[serde(flatten, default)]
    pub extra: std::collections::HashMap<String, serde_json::Value>,
}

/// OpenAI-compatible chat completion response.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatCompletionResponse {
    pub id: String,
    /// Object type. Optional because Claude models via Copilot API omit it.
    #[serde(default = "default_object_type")]
    pub object: String,
    pub created: i64,
    pub model: String,
    pub choices: Vec<Choice>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub usage: Option<Usage>,
}

fn default_object_type() -> String {
    "chat.completion".to_string()
}

/// A model object in OpenAI format.
///
/// Note: The `created` and `owned_by` fields are optional because GitHub Copilot's
/// `/models` API returns a different schema that omits these fields.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Model {
    pub id: String,
    pub object: String,
    #[serde(default)]
    pub created: i64,
    #[serde(default = "default_owned_by")]
    pub owned_by: String,
}

fn default_owned_by() -> String {
    "github-copilot".to_string()
}

/// Response for the `/v1/models` list endpoint.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelList {
    pub object: String,
    pub data: Vec<Model>,
}

// --- Streaming (SSE) types ---

/// Delta content within a streaming chunk choice.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChunkDelta {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub role: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,
    /// Tool calls being generated (streaming format with index for parallel calls).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<Vec<StreamingToolCall>>,
}

/// A single choice in a streaming chat completion chunk.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChunkChoice {
    pub index: u32,
    pub delta: ChunkDelta,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub finish_reason: Option<String>,
}

/// A streaming chat completion chunk (OpenAI-compatible SSE format).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatCompletionChunk {
    pub id: String,
    /// Object type. Optional because Claude models via Copilot API omit it.
    #[serde(default = "default_chunk_object_type")]
    pub object: String,
    pub created: i64,
    pub model: String,
    pub choices: Vec<ChunkChoice>,
}

fn default_chunk_object_type() -> String {
    "chat.completion.chunk".to_string()
}

// ---------------------------------------------------------------------------
// Streaming tool call types for native function calling
// ---------------------------------------------------------------------------

/// Tool call in a streaming delta.
///
/// Unlike the non-streaming `ToolCall` (from `tools::types`), this type includes
/// an `index` field for tracking parallel tool calls across chunks, and all fields
/// except `index` are optional since each chunk carries only partial data.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StreamingToolCall {
    /// Index of this tool call (for parallel calls).
    pub index: u32,
    /// Tool call ID (only present on first chunk for this call).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    /// Tool call type (only present on first chunk).
    #[serde(rename = "type", skip_serializing_if = "Option::is_none")]
    pub call_type: Option<String>,
    /// Function details.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub function: Option<StreamingFunctionCall>,
}

/// Function call details in a streaming delta.
///
/// Both fields are optional because subsequent chunks may contain only
/// partial `arguments` data without repeating the function `name`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StreamingFunctionCall {
    /// Function name (only present on first chunk for this call).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// Partial arguments JSON string.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub arguments: Option<String>,
}

// ---------------------------------------------------------------------------
// OpenAI-format tool definitions for native function calling
// ---------------------------------------------------------------------------

/// OpenAI-format tool definition for native function calling.
///
/// Separate from `crate::tools::types::Tool` which is used for the prompt-injection path.
/// These types are used when forwarding tool definitions natively to the Copilot/OpenAI API.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OpenAITool {
    #[serde(rename = "type")]
    pub tool_type: String,
    pub function: OpenAIToolFunction,
}

/// Function definition within an OpenAI tool (native function calling).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OpenAIToolFunction {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parameters: Option<serde_json::Value>,
}
