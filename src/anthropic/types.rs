use serde::{Deserialize, Serialize};

use crate::copilot::types::{
    ChatCompletionRequest, ChatCompletionResponse, Message, MessageContent,
};
use crate::tools::types::{Function, Tool, ToolCall};

// ---------------------------------------------------------------------------
// Anthropic shared types (image, document, cache control)
// ---------------------------------------------------------------------------

/// Source for an image content block.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum ImageSource {
    #[serde(rename = "base64")]
    Base64 {
        media_type: String,
        data: String,
    },
    #[serde(rename = "url")]
    Url {
        #[serde(skip_serializing_if = "Option::is_none")]
        media_type: Option<String>,
        url: String,
    },
}

/// Source for a document content block.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum DocumentSource {
    #[serde(rename = "base64")]
    Base64 {
        media_type: String,
        data: String,
    },
    #[serde(rename = "text")]
    Text {
        media_type: String,
        data: String,
    },
    #[serde(rename = "url")]
    Url {
        #[serde(skip_serializing_if = "Option::is_none")]
        media_type: Option<String>,
        url: String,
    },
}

/// Cache control hints for content blocks.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CacheControl {
    #[serde(rename = "type")]
    pub cache_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ttl: Option<u32>,
}

// ---------------------------------------------------------------------------
// Anthropic request types
// ---------------------------------------------------------------------------

/// Content within an Anthropic message — either plain text or a structured block.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ContentBlockInput {
    /// Plain text string content.
    Text(String),
    /// Array of typed content blocks (e.g. `[{"type":"text","text":"..."}]`).
    Blocks(Vec<ContentBlock>),
}

/// System prompt input — either a plain string or an array of content blocks.
/// The Anthropic API accepts both formats for the `system` field.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum SystemInput {
    /// Plain text string system prompt.
    Text(String),
    /// Array of typed content blocks (e.g. `[{"type":"text","text":"..."}]`).
    Blocks(Vec<ContentBlock>),
}

impl SystemInput {
    /// Extract the plain text content from the system input.
    pub fn to_text(&self) -> String {
        match self {
            SystemInput::Text(s) => s.clone(),
            SystemInput::Blocks(blocks) => blocks
                .iter()
                .filter_map(|b| match b {
                    ContentBlock::Text { text, .. } => Some(text.as_str()),
                    _ => None,
                })
                .collect::<Vec<_>>()
                .join(""),
        }
    }
}

/// A single content block within a message.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum ContentBlock {
    #[serde(rename = "text")]
    Text {
        text: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        cache_control: Option<CacheControl>,
    },
    #[serde(rename = "image")]
    Image {
        source: ImageSource,
        #[serde(skip_serializing_if = "Option::is_none")]
        cache_control: Option<CacheControl>,
    },
    #[serde(rename = "document")]
    Document {
        source: DocumentSource,
        #[serde(skip_serializing_if = "Option::is_none")]
        title: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        cache_control: Option<CacheControl>,
    },
    #[serde(rename = "tool_use")]
    ToolUse {
        id: String,
        name: String,
        input: serde_json::Value,
        #[serde(skip_serializing_if = "Option::is_none")]
        cache_control: Option<CacheControl>,
    },
    #[serde(rename = "tool_result")]
    ToolResult {
        tool_use_id: String,
        content: ToolResultContent,
        #[serde(skip_serializing_if = "Option::is_none")]
        cache_control: Option<CacheControl>,
    },
}

/// Content within a tool_result block — either a plain string or nested blocks.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ToolResultContent {
    Text(String),
    Blocks(Vec<ContentBlock>),
}

/// An Anthropic-format chat message.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AnthropicMessage {
    pub role: String,
    pub content: ContentBlockInput,
}

/// Anthropic-format tool definition.
///
/// Schema: `{ name, description?, input_schema: { type: "object", properties, required? } }`
///
/// NOTE: `input_schema` is technically required by the Anthropic API spec, but we make it
/// optional here to gracefully handle malformed requests from clients. When missing, we
/// provide a default empty object schema.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolDefinition {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default = "default_input_schema")]
    pub input_schema: InputSchema,
}

/// Provides a default input schema (empty object) for tools that don't specify one.
fn default_input_schema() -> InputSchema {
    InputSchema {
        schema_type: "object".to_string(),
        properties: None,
        required: None,
    }
}

/// JSON Schema describing a tool's input parameters in Anthropic format.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InputSchema {
    #[serde(rename = "type")]
    pub schema_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub properties: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub required: Option<Vec<String>>,
}

impl ToolDefinition {
    /// Convert an Anthropic `ToolDefinition` to the internal `Tool` format
    /// used by the prompt injector.
    pub fn to_internal_tool(&self) -> Tool {
        // Build the parameters JSON object from the InputSchema fields.
        let mut params = serde_json::Map::new();
        params.insert(
            "type".into(),
            serde_json::Value::String(self.input_schema.schema_type.clone()),
        );
        if let Some(ref props) = self.input_schema.properties {
            params.insert("properties".into(), props.clone());
        }
        if let Some(ref req) = self.input_schema.required {
            params.insert(
                "required".into(),
                serde_json::Value::Array(
                    req.iter()
                        .map(|s| serde_json::Value::String(s.clone()))
                        .collect(),
                ),
            );
        }

        Tool {
            tool_type: "function".to_string(),
            function: Function {
                name: self.name.clone(),
                description: self.description.clone(),
                parameters: Some(serde_json::Value::Object(params)),
            },
        }
    }
}

/// Anthropic Messages API request body.
///
/// **Note on `tool_choice`:** The Anthropic API supports a `tool_choice` field
/// that controls whether/how tools are used (e.g., `"auto"`, `"required"`,
/// `{"type": "tool", "name": "..."}`). This field is accepted but **ignored**
/// because the adapter uses prompt injection for tool support — the upstream
/// Copilot API has no native tool calling, so tool choice cannot be enforced.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AnthropicRequest {
    pub model: String,
    pub max_tokens: u32,
    pub messages: Vec<AnthropicMessage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub system: Option<SystemInput>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stream: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub top_p: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stop_sequences: Option<Vec<String>>,
    /// Tool definitions (not forwarded to Copilot API; used for prompt injection).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tools: Option<Vec<ToolDefinition>>,
    /// Tool choice preference. Accepted for API compatibility but **not enforced**
    /// — the adapter uses prompt injection and cannot control the model's tool
    /// calling behaviour at the API level.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_choice: Option<serde_json::Value>,
}

// ---------------------------------------------------------------------------
// Anthropic response types
// ---------------------------------------------------------------------------

/// A content block in the response.
///
/// This uses `#[serde(untagged)]` so that we can serialise both text and
/// tool_use blocks with the correct shape (text has `type`+`text`, tool_use
/// has `type`+`id`+`name`+`input`).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ResponseContentBlock {
    Text {
        #[serde(rename = "type")]
        block_type: String,
        text: String,
    },
    ToolUse {
        #[serde(rename = "type")]
        block_type: String,
        id: String,
        name: String,
        input: serde_json::Value,
    },
}

impl ResponseContentBlock {
    /// Create a text content block.
    pub fn text(text: String) -> Self {
        ResponseContentBlock::Text {
            block_type: "text".to_string(),
            text,
        }
    }

    /// Create a tool_use content block from a parsed `ToolCall`.
    pub fn tool_use(tool_call: &ToolCall) -> Self {
        let input = tool_call
            .function
            .arguments
            .as_ref()
            .and_then(|args| serde_json::from_str(args).ok())
            .unwrap_or(serde_json::Value::Object(serde_json::Map::new()));

        ResponseContentBlock::ToolUse {
            block_type: "tool_use".to_string(),
            id: tool_call
                .id
                .clone()
                .unwrap_or_else(|| "call_unknown".to_string()),
            name: tool_call
                .function
                .name
                .clone()
                .unwrap_or_default(),
            input,
        }
    }

    /// Get the block type string.
    ///
    /// The `block_type` field is set by the constructor methods (`text()`,
    /// `tool_use()`) and must match the enum variant. Direct struct
    /// construction is discouraged — always use the constructors.
    pub fn block_type(&self) -> &str {
        match self {
            ResponseContentBlock::Text { block_type, .. } => {
                debug_assert_eq!(
                    block_type, "text",
                    "Text variant block_type must be \"text\""
                );
                block_type
            }
            ResponseContentBlock::ToolUse { block_type, .. } => {
                debug_assert_eq!(
                    block_type, "tool_use",
                    "ToolUse variant block_type must be \"tool_use\""
                );
                block_type
            }
        }
    }

    /// Get the text content. Returns empty string for non-text blocks.
    pub fn text_content(&self) -> &str {
        match self {
            ResponseContentBlock::Text { text, .. } => text,
            ResponseContentBlock::ToolUse { .. } => "",
        }
    }
}

/// Token usage in Anthropic format.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AnthropicUsage {
    pub input_tokens: u32,
    pub output_tokens: u32,
}

/// Anthropic Messages API response.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AnthropicResponse {
    pub id: String,
    #[serde(rename = "type")]
    pub response_type: String,
    pub role: String,
    pub content: Vec<ResponseContentBlock>,
    pub model: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stop_reason: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stop_sequence: Option<String>,
    pub usage: AnthropicUsage,
}

// ---------------------------------------------------------------------------
// Anthropic streaming event types
// ---------------------------------------------------------------------------

/// Top-level streaming event wrapper.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum StreamEvent {
    #[serde(rename = "message_start")]
    MessageStart { message: AnthropicResponse },

    #[serde(rename = "content_block_start")]
    ContentBlockStart {
        index: u32,
        content_block: ResponseContentBlock,
    },

    #[serde(rename = "content_block_delta")]
    ContentBlockDelta { index: u32, delta: ContentDelta },

    #[serde(rename = "content_block_stop")]
    ContentBlockStop { index: u32 },

    #[serde(rename = "message_delta")]
    MessageDelta {
        delta: MessageDeltaBody,
        usage: MessageDeltaUsage,
    },

    #[serde(rename = "message_stop")]
    MessageStop {},

    #[serde(rename = "ping")]
    Ping {},
}

/// Delta payload for `content_block_delta` events carrying text.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TextDelta {
    #[serde(rename = "type")]
    pub delta_type: String,
    pub text: String,
}

/// Delta payload for `content_block_delta` events carrying tool input JSON.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InputJsonDelta {
    #[serde(rename = "type")]
    pub delta_type: String,
    pub partial_json: String,
}

/// Unified delta payload for `content_block_delta` events.
///
/// Covers both text deltas (for text content blocks) and input JSON deltas
/// (for tool_use content blocks during streaming).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ContentDelta {
    Text(TextDelta),
    InputJson(InputJsonDelta),
}

/// Delta body for `message_delta` events.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MessageDeltaBody {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stop_reason: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stop_sequence: Option<String>,
}

/// Usage info attached to `message_delta` events.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MessageDeltaUsage {
    pub output_tokens: u32,
}

// ---------------------------------------------------------------------------
// Request translation: Anthropic → OpenAI
// ---------------------------------------------------------------------------

/// Extract plain text from an Anthropic content block input.
fn extract_text(content: &ContentBlockInput) -> String {
    match content {
        ContentBlockInput::Text(s) => s.clone(),
        ContentBlockInput::Blocks(blocks) => blocks
            .iter()
            .filter_map(|b| match b {
                ContentBlock::Text { text, .. } => Some(text.clone()),
                ContentBlock::Image { .. } => Some("[Image]".to_string()),
                ContentBlock::Document { title, .. } => {
                    Some(title.clone().unwrap_or_else(|| "[Document]".to_string()))
                }
                _ => None,
            })
            .collect::<Vec<_>>()
            .join(""),
    }
}

/// Check if the content blocks contain any multimodal blocks (image or document).
fn has_multimodal_blocks(content: &ContentBlockInput) -> bool {
    match content {
        ContentBlockInput::Text(_) => false,
        ContentBlockInput::Blocks(blocks) => blocks
            .iter()
            .any(|b| matches!(b, ContentBlock::Image { .. } | ContentBlock::Document { .. })),
    }
}

/// Translate an Anthropic content block to an OpenAI content block.
///
/// Returns `None` for blocks that cannot be represented in OpenAI format
/// (e.g., documents), which should be skipped via `filter_map`.
fn translate_content_block(
    block: &ContentBlock,
) -> Option<crate::copilot::types::ContentBlock> {
    use crate::copilot::types;
    match block {
        ContentBlock::Text { text, .. } => {
            Some(types::ContentBlock::Text { text: text.clone() })
        }
        ContentBlock::Image { source, .. } => {
            let url = match source {
                ImageSource::Base64 { media_type, data } => {
                    format!("data:{};base64,{}", media_type, data)
                }
                ImageSource::Url { url, .. } => url.clone(),
            };
            Some(types::ContentBlock::ImageUrl {
                image_url: types::ImageUrl {
                    url,
                    detail: None,
                },
            })
        }
        ContentBlock::Document { title, .. } => {
            tracing::warn!(
                title = title.as_deref(),
                "Document content blocks are not supported by OpenAI format; skipping"
            );
            None
        }
        // ToolUse and ToolResult blocks are handled by other translation paths
        _ => None,
    }
}

/// Check if the content blocks contain any `tool_result` blocks.
fn has_tool_result_blocks(content: &ContentBlockInput) -> bool {
    match content {
        ContentBlockInput::Text(_) => false,
        ContentBlockInput::Blocks(blocks) => blocks
            .iter()
            .any(|b| matches!(b, ContentBlock::ToolResult { .. })),
    }
}

/// Extract `tool_result` blocks from content and return them as
/// OpenAI-format `tool` role messages.
fn extract_tool_result_messages(content: &ContentBlockInput) -> Vec<Message> {
    match content {
        ContentBlockInput::Text(_) => vec![],
        ContentBlockInput::Blocks(blocks) => blocks
            .iter()
            .filter_map(|b| match b {
                ContentBlock::ToolResult {
                    tool_use_id,
                    content,
                    ..
                } => {
                    let text = match content {
                        ToolResultContent::Text(s) => s.clone(),
                        ToolResultContent::Blocks(inner) => inner
                            .iter()
                            .filter_map(|ib| match ib {
                                ContentBlock::Text { text, .. } => Some(text.as_str()),
                                _ => None,
                            })
                            .collect::<Vec<_>>()
                            .join(""),
                    };
                    Some(Message {
                        role: "tool".to_string(),
                        content: MessageContent::Text(text),
                        name: None,
                        tool_calls: None,
                        tool_call_id: Some(tool_use_id.clone()),
                    })
                }
                _ => None,
            })
            .collect(),
    }
}

impl AnthropicRequest {
    /// Convert an Anthropic Messages API request into an OpenAI-compatible
    /// `ChatCompletionRequest`.
    ///
    /// - The optional `system` field is prepended as a system message.
    /// - Content blocks are flattened to plain text.
    /// - `tool_result` content blocks are translated to `tool` role messages.
    /// - `stop_sequences` maps to the OpenAI `stop` field.
    pub fn to_chat_completion_request(&self) -> ChatCompletionRequest {
        let mut messages = Vec::new();

        // Prepend system prompt as a system message
        if let Some(system) = &self.system {
            messages.push(Message {
                role: "system".to_string(),
                content: MessageContent::Text(system.to_text()),
                name: None,
                tool_calls: None,
                tool_call_id: None,
            });
        }

        // Convert each Anthropic message
        for msg in &self.messages {
            // Check if this message contains tool_result blocks.
            // If so, extract them as separate tool-role messages.
            // Note: tool-role messages are inserted *before* any remaining
            // text from the same message. This reorders content when the
            // original message mixes text and tool_result blocks, but this
            // is acceptable because the Anthropic API does not typically
            // combine text and tool_result in the same user message.
            if has_tool_result_blocks(&msg.content) {
                let tool_messages = extract_tool_result_messages(&msg.content);
                messages.extend(tool_messages);

                // Also include any non-tool-result text as a regular message
                let text = extract_text(&msg.content);
                if !text.is_empty() {
                    messages.push(Message {
                        role: msg.role.clone(),
                        content: MessageContent::Text(text),
                        name: None,
                        tool_calls: None,
                        tool_call_id: None,
                    });
                }
            } else if has_multimodal_blocks(&msg.content) {
                // Message contains image or document blocks — build multimodal
                // content with OpenAI-format content blocks.
                if let ContentBlockInput::Blocks(blocks) = &msg.content {
                    let translated: Vec<crate::copilot::types::ContentBlock> = blocks
                        .iter()
                        .filter_map(|b| translate_content_block(b))
                        .collect();
                    if !translated.is_empty() {
                        messages.push(Message {
                            role: msg.role.clone(),
                            content: MessageContent::Blocks(translated),
                            name: None,
                            tool_calls: None,
                            tool_call_id: None,
                        });
                    }
                }
            } else {
                // Known limitation: assistant messages containing `ToolUse` content
                // blocks (from a prior turn's tool invocation) are reduced to
                // text-only here — the `ToolUse` variants are discarded by
                // `extract_text`. In multi-turn tool conversations the upstream
                // model loses context about which tool was called. This is an
                // inherent limitation of the prompt-injection approach: the
                // Copilot API has no native tool calling, so there is no way to
                // represent tool_use blocks in the upstream request format.
                messages.push(Message {
                    role: msg.role.clone(),
                    content: MessageContent::Text(extract_text(&msg.content)),
                    name: None,
                    tool_calls: None,
                    tool_call_id: None,
                });
            }
        }

        let stop = self.stop_sequences.as_ref().map(|seqs| {
            serde_json::Value::Array(
                seqs.iter()
                    .map(|s| serde_json::Value::String(s.clone()))
                    .collect(),
            )
        });

        ChatCompletionRequest {
            model: crate::model_mapper::normalize_model_name(&self.model),
            messages,
            stream: self.stream,
            temperature: self.temperature,
            max_tokens: Some(self.max_tokens),
            top_p: self.top_p,
            n: None,
            stop,
            presence_penalty: None,
            frequency_penalty: None,
            tools: None,
            tool_choice: None,
        }
    }
}

// ---------------------------------------------------------------------------
// Response translation: OpenAI → Anthropic
// ---------------------------------------------------------------------------

/// Map an OpenAI `finish_reason` to an Anthropic `stop_reason`.
pub fn map_stop_reason(finish_reason: Option<&str>) -> Option<String> {
    finish_reason.map(|r| match r {
        "stop" => "end_turn".to_string(),
        "length" => "max_tokens".to_string(),
        other => other.to_string(),
    })
}

impl ChatCompletionResponse {
    /// Convert an OpenAI chat completion response into an Anthropic-format
    /// `AnthropicResponse`.
    ///
    /// When the first choice contains `tool_calls`, they are returned as
    /// `tool_use` content blocks and `stop_reason` is set to `"tool_use"`.
    pub fn to_anthropic_response(&self) -> AnthropicResponse {
        let first_choice = self.choices.first();

        let content_text = first_choice
            .map(|c| c.message.content.as_text())
            .unwrap_or_default();

        let has_tool_calls = first_choice
            .and_then(|c| c.message.tool_calls.as_ref())
            .map_or(false, |tc| !tc.is_empty());

        let stop_reason = if has_tool_calls {
            Some("tool_use".to_string())
        } else {
            first_choice.and_then(|c| map_stop_reason(c.finish_reason.as_deref()))
        };

        let usage = self.usage.as_ref().map(|u| AnthropicUsage {
            input_tokens: u.prompt_tokens,
            output_tokens: u.completion_tokens,
        }).unwrap_or(AnthropicUsage {
            input_tokens: 0,
            output_tokens: 0,
        });

        // Build content blocks
        let content = if self.choices.is_empty() {
            vec![]
        } else {
            let mut blocks = Vec::new();

            // Add text block if there is any text content
            if !content_text.is_empty() {
                blocks.push(ResponseContentBlock::text(content_text));
            }

            // Add tool_use blocks for any tool calls
            if let Some(tool_calls) = first_choice.and_then(|c| c.message.tool_calls.as_ref()) {
                for tc in tool_calls {
                    blocks.push(ResponseContentBlock::tool_use(tc));
                }
            }

            // If no text and no tool calls, still return an empty text block
            if blocks.is_empty() {
                blocks.push(ResponseContentBlock::text(String::new()));
            }

            blocks
        };

        AnthropicResponse {
            // Strip OpenAI "chatcmpl-" prefix if present; if absent, use the raw ID.
            id: format!("msg_{}", self.id.trim_start_matches("chatcmpl-")),
            response_type: "message".to_string(),
            role: "assistant".to_string(),
            content,
            model: self.model.clone(),
            stop_reason,
            stop_sequence: None,
            usage,
        }
    }
}

/// Build the initial `AnthropicResponse` shell used in the `message_start`
/// streaming event. Usage starts at zero and content is empty; they will be
/// filled in by subsequent delta events.
pub fn build_message_start_response(id: &str, model: &str) -> AnthropicResponse {
    AnthropicResponse {
        // Strip OpenAI "chatcmpl-" prefix if present; if absent, use the raw ID.
        id: format!("msg_{}", id.trim_start_matches("chatcmpl-")),
        response_type: "message".to_string(),
        role: "assistant".to_string(),
        content: vec![],
        model: model.to_string(),
        stop_reason: None,
        stop_sequence: None,
        usage: AnthropicUsage {
            input_tokens: 0,
            output_tokens: 0,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_text_from_string() {
        let input = ContentBlockInput::Text("hello".into());
        assert_eq!(extract_text(&input), "hello");
    }

    #[test]
    fn extract_text_from_blocks() {
        let input = ContentBlockInput::Blocks(vec![
            ContentBlock::Text { text: "Hello ".into(), cache_control: None },
            ContentBlock::Text { text: "world".into(), cache_control: None },
        ]);
        assert_eq!(extract_text(&input), "Hello world");
    }

    #[test]
    fn system_input_from_string() {
        let input = SystemInput::Text("You are helpful".into());
        assert_eq!(input.to_text(), "You are helpful");
    }

    #[test]
    fn system_input_from_blocks() {
        let input = SystemInput::Blocks(vec![
            ContentBlock::Text { text: "You are ".into(), cache_control: None },
            ContentBlock::Text { text: "helpful".into(), cache_control: None },
        ]);
        assert_eq!(input.to_text(), "You are helpful");
    }

    #[test]
    fn deserialize_system_as_string() {
        let json = r#"{"model":"claude-3","max_tokens":1024,"messages":[],"system":"Hello"}"#;
        let req: AnthropicRequest = serde_json::from_str(json).unwrap();
        assert_eq!(req.system.unwrap().to_text(), "Hello");
    }

    #[test]
    fn deserialize_system_as_blocks() {
        let json = r#"{"model":"claude-3","max_tokens":1024,"messages":[],"system":[{"type":"text","text":"Hello"}]}"#;
        let req: AnthropicRequest = serde_json::from_str(json).unwrap();
        assert_eq!(req.system.unwrap().to_text(), "Hello");
    }
}
