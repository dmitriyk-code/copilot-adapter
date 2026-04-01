use tiktoken_rs::cl100k_base;

use crate::anthropic::types::{
    ContentBlock, ContentBlockInput, CountTokensRequest, SystemInput, ToolResultContent,
};

/// Fixed token estimate for image and document blocks (approximates a low-res
/// image tile). The design document originally specified 100 tokens for Document
/// blocks, but we use the same 85-token estimate for consistency — both image
/// and document blocks represent opaque media whose actual token usage varies
/// widely and is not directly tokenizable by the BPE encoder.
const IMAGE_TOKEN_ESTIMATE: usize = 85;

/// Per-message overhead tokens (standard tiktoken convention).
const MESSAGE_OVERHEAD: usize = 4;

/// Errors that can occur during token counting.
#[derive(Debug, thiserror::Error)]
pub enum TokenCountError {
    #[error("Failed to initialize tokenizer: {0}")]
    EncoderInit(String),
    #[error("Failed to serialize: {0}")]
    Serialization(String),
}

/// Count tokens for a CountTokensRequest using tiktoken cl100k_base.
///
/// Includes tokens from the system prompt, all messages (with per-message
/// overhead), and tool definitions. Image and document blocks use a fixed estimate.
///
/// NOTE: `cl100k_base()` is called on every invocation, which loads the BPE
/// vocabulary data each time. This is an accepted trade-off for simplicity —
/// see Open Question 4 in the design document (ROOT-AND-COUNT-TOKENS.plan.md).
/// TODO: If profiling shows this becomes a bottleneck at high request rates,
/// consider caching the encoder in AppState behind an Arc.
pub fn count_tokens(request: &CountTokensRequest) -> Result<u32, TokenCountError> {
    let bpe =
        cl100k_base().map_err(|e| TokenCountError::EncoderInit(e.to_string()))?;

    let mut total: usize = 0;

    // System prompt
    if let Some(system) = &request.system {
        total += count_system_tokens(&bpe, system)?;
    }

    // Messages
    for msg in &request.messages {
        total += MESSAGE_OVERHEAD;
        total += count_content_tokens(&bpe, &msg.content)?;
    }

    // Tool definitions
    if let Some(tools) = &request.tools {
        for tool in tools {
            let json = serde_json::to_string(tool)
                .map_err(|e| TokenCountError::Serialization(e.to_string()))?;
            total += bpe.encode_with_special_tokens(&json).len();
        }
    }

    Ok(total as u32)
}

/// Count tokens for a system prompt input.
fn count_system_tokens(
    bpe: &tiktoken_rs::CoreBPE,
    system: &SystemInput,
) -> Result<usize, TokenCountError> {
    match system {
        SystemInput::Text(s) => Ok(bpe.encode_with_special_tokens(s).len()),
        SystemInput::Blocks(blocks) => {
            let mut total = 0;
            for block in blocks {
                total += count_block_tokens(bpe, block)?;
            }
            Ok(total)
        }
    }
}

/// Count tokens for a message's content (plain text string or array of blocks).
fn count_content_tokens(
    bpe: &tiktoken_rs::CoreBPE,
    content: &ContentBlockInput,
) -> Result<usize, TokenCountError> {
    match content {
        ContentBlockInput::Text(s) => Ok(bpe.encode_with_special_tokens(s).len()),
        ContentBlockInput::Blocks(blocks) => {
            let mut total = 0;
            for block in blocks {
                total += count_block_tokens(bpe, block)?;
            }
            Ok(total)
        }
    }
}

/// Count tokens for a single content block.
fn count_block_tokens(
    bpe: &tiktoken_rs::CoreBPE,
    block: &ContentBlock,
) -> Result<usize, TokenCountError> {
    match block {
        ContentBlock::Text { text, .. } => {
            Ok(bpe.encode_with_special_tokens(text).len())
        }
        ContentBlock::Image { .. } => Ok(IMAGE_TOKEN_ESTIMATE),
        ContentBlock::Document { .. } => Ok(IMAGE_TOKEN_ESTIMATE),
        ContentBlock::ToolUse {
            id, name, input, ..
        } => {
            let json = serde_json::to_string(&serde_json::json!({
                "type": "tool_use",
                "id": id,
                "name": name,
                "input": input,
            }))
            .map_err(|e| TokenCountError::Serialization(e.to_string()))?;
            Ok(bpe.encode_with_special_tokens(&json).len())
        }
        ContentBlock::ToolResult {
            tool_use_id,
            content,
            ..
        } => {
            let content_text = match content {
                ToolResultContent::Text(s) => s.clone(),
                ToolResultContent::Blocks(blocks) => {
                    let mut text = String::new();
                    for b in blocks {
                        match b {
                            ContentBlock::Text { text: t, .. } => text.push_str(t),
                            _ => {
                                // NOTE: Non-text blocks (e.g. Image) inside ToolResult are
                                // serialized to JSON rather than using IMAGE_TOKEN_ESTIMATE.
                                // This intentionally counts full serialized content (including
                                // base64 data) and may overcount for large payloads. See
                                // docs/known-issues.md or file a follow-up to unify with
                                // top-level block handling.
                                let json = serde_json::to_string(b)
                                    .map_err(|e| {
                                        TokenCountError::Serialization(e.to_string())
                                    })?;
                                text.push_str(&json);
                            }
                        }
                    }
                    text
                }
            };
            let json = serde_json::to_string(&serde_json::json!({
                "type": "tool_result",
                "tool_use_id": tool_use_id,
                "content": content_text,
            }))
            .map_err(|e| TokenCountError::Serialization(e.to_string()))?;
            Ok(bpe.encode_with_special_tokens(&json).len())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::anthropic::types::AnthropicMessage;

    fn make_text_request(text: &str) -> CountTokensRequest {
        CountTokensRequest {
            model: "claude-sonnet-4-20250514".to_string(),
            messages: vec![AnthropicMessage {
                role: "user".to_string(),
                content: ContentBlockInput::Text(text.to_string()),
            }],
            system: None,
            tools: None,
        }
    }

    #[test]
    fn count_tokens_simple_text() {
        let req = make_text_request("Hello, world!");
        let count = count_tokens(&req).unwrap();
        // "Hello, world!" is ~4 tokens + 4 message overhead = ~8
        assert!(count > 0, "Token count should be > 0, got {count}");
        assert!(count < 50, "Simple text should be < 50 tokens, got {count}");
    }

    #[test]
    fn count_tokens_empty_text() {
        let req = make_text_request("");
        let count = count_tokens(&req).unwrap();
        // Empty text = 0 text tokens + 4 overhead = 4
        assert_eq!(count, 4, "Empty text should be 4 (overhead only), got {count}");
    }

    #[test]
    fn count_tokens_returns_nonzero_for_text() {
        let req = make_text_request("The quick brown fox jumps over the lazy dog.");
        let count = count_tokens(&req).unwrap();
        assert!(count > 4, "Should count text tokens beyond overhead");
    }
}
