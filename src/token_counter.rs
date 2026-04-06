use tiktoken_rs::cl100k_base;

use crate::anthropic::types::{
    AnthropicRequest, ContentBlock, ContentBlockInput, CountTokensRequest, SystemInput,
    ToolResultContent,
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

/// Count input tokens for an [`AnthropicRequest`].
///
/// Reuses the same `cl100k_base` BPE encoder as the `count_tokens` endpoint.
/// Returns 0 on encoder failure — token counting is best-effort and must not
/// block the request.
pub fn count_tokens_for_request(request: &AnthropicRequest) -> u32 {
    let count_request = CountTokensRequest {
        model: request.model.clone(),
        messages: request.messages.clone(),
        system: request.system.clone(),
        tools: request.tools.clone(),
    };
    count_tokens(&count_request).unwrap_or(0)
}

/// Count output tokens for a completed response string.
///
/// Tokenizes `text` using `cl100k_base`. Returns 0 on encoder failure.
pub fn count_output_tokens(text: &str) -> u32 {
    match cl100k_base() {
        Ok(bpe) => bpe.encode_with_special_tokens(text).len() as u32,
        Err(_) => 0,
    }
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
        // Thinking blocks are internal reasoning artifacts; they contribute
        // negligible tokens to the prompt since they're stripped during
        // translation. Count zero tokens.
        ContentBlock::Thinking { .. } | ContentBlock::RedactedThinking { .. } => Ok(0),
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

    // -----------------------------------------------------------------------
    // count_tokens_for_request tests
    // -----------------------------------------------------------------------

    fn make_anthropic_request(text: &str) -> AnthropicRequest {
        AnthropicRequest {
            model: "claude-sonnet-4-20250514".to_string(),
            max_tokens: 1024,
            messages: vec![AnthropicMessage {
                role: "user".to_string(),
                content: ContentBlockInput::Text(text.to_string()),
            }],
            system: None,
            stream: None,
            temperature: None,
            top_p: None,
            stop_sequences: None,
            tools: None,
            tool_choice: None,
            output_config: None,
            thinking: None,
        }
    }

    #[test]
    fn count_tokens_for_request_matches_count_tokens() {
        let text = "Hello, world!";
        let anthropic_req = make_anthropic_request(text);
        let count_req = make_text_request(text);

        let from_request = count_tokens_for_request(&anthropic_req);
        let from_count = count_tokens(&count_req).unwrap();
        assert_eq!(
            from_request, from_count,
            "count_tokens_for_request should match count_tokens"
        );
    }

    #[test]
    fn count_tokens_for_request_with_system_prompt() {
        let mut req = make_anthropic_request("test");
        req.system = Some(SystemInput::Text("You are a helpful assistant.".to_string()));

        let count = count_tokens_for_request(&req);
        // Should include system prompt tokens + message tokens + overhead
        assert!(count > 4, "Should include system prompt tokens, got {count}");
    }

    #[test]
    fn count_tokens_for_request_returns_nonzero() {
        let req = make_anthropic_request("The quick brown fox");
        let count = count_tokens_for_request(&req);
        assert!(count > 0, "Should return > 0 for non-empty request, got {count}");
    }

    // -----------------------------------------------------------------------
    // count_output_tokens tests
    // -----------------------------------------------------------------------

    #[test]
    fn count_output_tokens_empty_string() {
        assert_eq!(count_output_tokens(""), 0, "Empty string should be 0 tokens");
    }

    #[test]
    fn count_output_tokens_hello_world() {
        let count = count_output_tokens("Hello, world!");
        assert!(count > 0, "Non-empty text should produce > 0 tokens, got {count}");
    }

    #[test]
    fn count_output_tokens_longer_text() {
        let short = count_output_tokens("Hi");
        let long = count_output_tokens("The quick brown fox jumps over the lazy dog. This is a much longer sentence with more tokens.");
        assert!(long > short, "Longer text should produce more tokens: short={short}, long={long}");
    }
}
