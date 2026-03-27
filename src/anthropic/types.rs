use serde::{Deserialize, Serialize};

use crate::copilot::types::{
    ChatCompletionRequest, ChatCompletionResponse, Message,
};

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

/// A single content block within a message.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum ContentBlock {
    #[serde(rename = "text")]
    Text { text: String },
}

/// An Anthropic-format chat message.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AnthropicMessage {
    pub role: String,
    pub content: ContentBlockInput,
}

/// Anthropic Messages API request body.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AnthropicRequest {
    pub model: String,
    pub max_tokens: u32,
    pub messages: Vec<AnthropicMessage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub system: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stream: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub top_p: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stop_sequences: Option<Vec<String>>,
}

// ---------------------------------------------------------------------------
// Anthropic response types
// ---------------------------------------------------------------------------

/// A content block in the response.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResponseContentBlock {
    #[serde(rename = "type")]
    pub block_type: String,
    pub text: String,
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
    ContentBlockDelta { index: u32, delta: TextDelta },

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

/// Delta payload for `content_block_delta` events.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TextDelta {
    #[serde(rename = "type")]
    pub delta_type: String,
    pub text: String,
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
            .map(|b| match b {
                ContentBlock::Text { text } => text.as_str(),
            })
            .collect::<Vec<_>>()
            .join(""),
    }
}

impl AnthropicRequest {
    /// Convert an Anthropic Messages API request into an OpenAI-compatible
    /// `ChatCompletionRequest`.
    ///
    /// - The optional `system` field is prepended as a system message.
    /// - Content blocks are flattened to plain text.
    /// - `stop_sequences` maps to the OpenAI `stop` field.
    pub fn to_chat_completion_request(&self) -> ChatCompletionRequest {
        let mut messages = Vec::new();

        // Prepend system prompt as a system message
        if let Some(system) = &self.system {
            messages.push(Message {
                role: "system".to_string(),
                content: system.clone(),
                name: None,
            });
        }

        // Convert each Anthropic message
        for msg in &self.messages {
            messages.push(Message {
                role: msg.role.clone(),
                content: extract_text(&msg.content),
                name: None,
            });
        }

        let stop = self.stop_sequences.as_ref().map(|seqs| {
            serde_json::Value::Array(
                seqs.iter()
                    .map(|s| serde_json::Value::String(s.clone()))
                    .collect(),
            )
        });

        ChatCompletionRequest {
            model: self.model.clone(),
            messages,
            stream: self.stream,
            temperature: self.temperature,
            max_tokens: Some(self.max_tokens),
            top_p: self.top_p,
            n: None,
            stop,
            presence_penalty: None,
            frequency_penalty: None,
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
    pub fn to_anthropic_response(&self) -> AnthropicResponse {
        let content_text = self
            .choices
            .first()
            .map(|c| c.message.content.clone())
            .unwrap_or_default();

        let stop_reason = self
            .choices
            .first()
            .and_then(|c| map_stop_reason(c.finish_reason.as_deref()));

        let usage = self.usage.as_ref().map(|u| AnthropicUsage {
            input_tokens: u.prompt_tokens,
            output_tokens: u.completion_tokens,
        }).unwrap_or(AnthropicUsage {
            input_tokens: 0,
            output_tokens: 0,
        });

        // Produce an empty content array when there are no choices (per Anthropic spec),
        // otherwise wrap the text in a single content block.
        let content = if self.choices.is_empty() {
            vec![]
        } else {
            vec![ResponseContentBlock {
                block_type: "text".to_string(),
                text: content_text,
            }]
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
            ContentBlock::Text { text: "Hello ".into() },
            ContentBlock::Text { text: "world".into() },
        ]);
        assert_eq!(extract_text(&input), "Hello world");
    }
}
