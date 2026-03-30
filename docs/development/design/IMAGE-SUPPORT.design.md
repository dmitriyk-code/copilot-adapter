# Plan: Add Image and Document Content Block Support

## Context

Claude Code attempted to upload an image, resulting in a deserialization error:
```
Failed to deserialize the JSON body into the target type: messages[14].content:
data did not match any variant of untagged enum ContentBlockInput
```

**Validated Approach**: This implementation matches LiteLLM's proven working solution for Claude Code → GitHub Copilot with image uploads.

The issue is that the `copilot-adapter`'s Anthropic types implementation only supports three content block types:
1. `text` - text content
2. `tool_use` - tool usage blocks
3. `tool_result` - tool result blocks

However, the Anthropic Messages API supports **many more content block types**, most critically:
- **`image`** - for vision capabilities (base64 or URL sources)
- **`document`** - for PDF/text document processing

When Claude Code sends an image block, the adapter fails to deserialize it because the `ContentBlock` enum has no `Image` variant.

**Solution**: Add image/document content block support with translation to OpenAI format, enabling vision-capable models (GPT-4o, Claude 3.5, etc.) to receive images through the adapter.

## Problem Analysis

### Current Implementation
In `src/anthropic/types.rs`:
- `ContentBlock` enum (line 50-67) - only supports `text`, `tool_use`, `tool_result`
- `ContentBlockInput` enum (line 14-20) - untagged enum wrapping text or blocks
- Used in `AnthropicMessage.content` field

### Missing Support
Based on the Anthropic API documentation, we need to add support for:

**Priority 1 (Critical for vision):**
- `image` blocks with base64/URL sources
- `document` blocks for PDF/text processing

**Priority 2 (Extended features):**
- `thinking` blocks (extended thinking)
- `redacted_thinking` blocks
- `search_result` blocks
- `server_tool_use` blocks
- `tool_reference` blocks
- `container_upload` blocks
- Optional `cache_control` on all block types

### Translation Strategy

Since copilot-adapter translates Anthropic → OpenAI format before sending to GitHub Copilot:

1. **Image blocks**: Translate to OpenAI's multimodal message format
2. **Document blocks**: Skip with warning (no OpenAI equivalent)
3. **Other blocks**: Gracefully ignore unsupported blocks during translation

## Implementation Plan

### Phase 1: Add Image Content Block Types

**File: `src/anthropic/types.rs`**

1. Add image/document source type definitions:
   ```rust
   /// Source for image content blocks.
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
           media_type: String,
           url: String,
       },
   }

   /// Source for document content blocks.
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
           media_type: String,
           url: String,
       },
   }

   /// Cache control metadata (accepted but ignored during translation).
   #[derive(Debug, Clone, Serialize, Deserialize)]
   pub struct CacheControl {
       #[serde(rename = "type")]
       pub cache_type: String,  // "ephemeral"
       #[serde(skip_serializing_if = "Option::is_none")]
       pub ttl: Option<String>,  // "5m" or "1h"
   }
   ```

2. Extend `ContentBlock` enum with new variants:
   ```rust
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
           context: Option<String>,
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
   ```

   Note: Update existing `Text`, `ToolUse`, `ToolResult` variants to include optional `cache_control`

3. Update `extract_text()` helper to handle new block types:
   ```rust
   fn extract_text(content: &ContentBlockInput) -> String {
       match content {
           ContentBlockInput::Text(s) => s.clone(),
           ContentBlockInput::Blocks(blocks) => blocks
               .iter()
               .filter_map(|b| match b {
                   ContentBlock::Text { text, .. } => Some(text.as_str()),
                   ContentBlock::Image { .. } => {
                       // Images cannot be represented as text - use placeholder
                       Some("[Image]")
                   }
                   ContentBlock::Document { title, .. } => {
                       // Use title as placeholder if available
                       title.as_ref().map(|s| s.as_str()).or(Some("[Document]"))
                   }
                   _ => None,
               })
               .collect::<Vec<_>>()
               .join(""),
       }
   }
   ```

### Phase 2: Add OpenAI Multimodal Support

**File: `src/copilot/types.rs`**

1. Extend `ContentBlock` enum to support image_url:
   ```rust
   #[derive(Debug, Clone, Serialize, Deserialize)]
   #[serde(tag = "type", rename_all = "snake_case")]
   pub enum ContentBlock {
       Text {
           text: String,
       },
       #[serde(rename = "image_url")]
       ImageUrl {
           image_url: ImageUrl,
       },
       #[serde(other)]
       Other,
   }

   #[derive(Debug, Clone, Serialize, Deserialize)]
   pub struct ImageUrl {
       pub url: String,
       #[serde(skip_serializing_if = "Option::is_none")]
       pub detail: Option<String>,  // "low", "high", "auto"
   }
   ```

2. Update `MessageContent::as_text()` to skip image blocks:
   ```rust
   impl MessageContent {
       pub fn as_text(&self) -> String {
           match self {
               MessageContent::Text(s) => s.clone(),
               MessageContent::Blocks(blocks) => blocks
                   .iter()
                   .filter_map(|b| match b {
                       ContentBlock::Text { text } => Some(text.as_str()),
                       ContentBlock::ImageUrl { .. } => None,  // Skip images
                       ContentBlock::Other => None,
                   })
                   .collect::<Vec<_>>()
                   .join(""),
           }
       }
   }
   ```

### Phase 3: Implement Translation Logic

**File: `src/anthropic/types.rs`**

Update `AnthropicRequest::to_chat_completion_request()` to handle multimodal content:

```rust
impl AnthropicRequest {
    pub fn to_chat_completion_request(&self) -> ChatCompletionRequest {
        let mut messages = Vec::new();

        // Prepend system prompt as before...
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
            // Handle tool_result blocks as before...
            if has_tool_result_blocks(&msg.content) {
                let tool_messages = extract_tool_result_messages(&msg.content);
                messages.extend(tool_messages);

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
            } else {
                // Check if we need multimodal content
                let needs_multimodal = matches!(&msg.content, ContentBlockInput::Blocks(blocks)
                    if blocks.iter().any(|b| matches!(b, ContentBlock::Image { .. })));

                if needs_multimodal {
                    // Build OpenAI content blocks array
                    if let ContentBlockInput::Blocks(blocks) = &msg.content {
                        let openai_blocks: Vec<crate::copilot::types::ContentBlock> = blocks
                            .iter()
                            .filter_map(|b| translate_content_block(b))
                            .collect();

                        if !openai_blocks.is_empty() {
                            messages.push(Message {
                                role: msg.role.clone(),
                                content: MessageContent::Blocks(openai_blocks),
                                name: None,
                                tool_calls: None,
                                tool_call_id: None,
                            });
                        }
                    }
                } else {
                    // Simple text message - existing logic
                    messages.push(Message {
                        role: msg.role.clone(),
                        content: MessageContent::Text(extract_text(&msg.content)),
                        name: None,
                        tool_calls: None,
                        tool_call_id: None,
                    });
                }
            }
        }

        // Build request as before...
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
            tools: None,
            tool_choice: None,
        }
    }
}

/// Translate a single Anthropic content block to OpenAI format.
fn translate_content_block(block: &ContentBlock) -> Option<crate::copilot::types::ContentBlock> {
    use crate::copilot::types;

    match block {
        ContentBlock::Text { text, .. } => {
            Some(types::ContentBlock::Text {
                text: text.clone(),
            })
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
            // Documents not supported in OpenAI format - log warning and skip
            tracing::warn!(
                title = title.as_deref(),
                "Document content blocks are not supported by OpenAI format; skipping"
            );
            None
        }
        ContentBlock::ToolUse { .. } | ContentBlock::ToolResult { .. } => {
            // These are handled separately in the message conversion logic
            None
        }
    }
}
```

### Phase 4: Testing

**Unit tests** (`tests/unit/anthropic_types_tests.rs`):
1. Test deserialization of image blocks (base64 and URL)
2. Test deserialization of document blocks
3. Test mixed content (text + image)
4. Test cache_control fields
5. Test translation to OpenAI format

**Integration tests** (`tests/integration/`):
1. Send request with image block via `/v1/messages` endpoint
2. Verify proper deserialization (no 422 error)
3. Verify response handling

**Manual E2E test:**
1. Start adapter: `cargo run -- start --experimental-tools`
2. Send curl request with image block
3. Verify response

### Phase 5: Extended Support (Optional)

Add support for additional block types:
- `thinking` / `redacted_thinking`
- `search_result`
- `server_tool_use`
- `tool_reference`
- `container_upload`

Strategy: Add variants but gracefully skip during translation to OpenAI format.

## Critical Files

- `src/anthropic/types.rs` - Content block type definitions and translation logic
- `src/copilot/types.rs` - OpenAI format types (check multimodal support)
- `tests/unit/anthropic_types_tests.rs` - Unit tests for content blocks
- `tests/integration/messages_endpoint.rs` - Integration tests

## Design Decisions

**Confirmed Working**: LiteLLM successfully handles Claude Code → GitHub Copilot with image uploads using this exact translation approach. This validates the implementation strategy.

1. **Image Block Translation:**
   - **Input**: Anthropic format `{"type": "image", "source": {"type": "base64|url", "data": "...", "media_type": "..."}}`
   - **Output**: OpenAI format `{"type": "image_url", "image_url": {"url": "data:image/jpeg;base64,..."}}`
   - **Strategy**: Translate Anthropic image blocks → OpenAI image_url blocks
   - Base64 sources: Convert to data URI format (`data:{media_type};base64,{data}`)
   - URL sources: Use URL directly

2. **Document Handling:**
   - Anthropic supports document blocks for PDF/text processing
   - OpenAI format has no standard document equivalent
   - **Strategy**: Skip document blocks with warning log (graceful degradation)
   - Future consideration: Extract text from text-type documents

3. **Cache Control:**
   - Anthropic supports `cache_control` on all block types
   - OpenAI/Copilot API does not support caching
   - **Strategy**: Accept but silently ignore during translation
   - Maintains API compatibility with Anthropic clients

4. **Unknown Content Blocks:**
   - Many Anthropic-specific blocks (thinking, search_result, etc.) have no OpenAI equivalent
   - **Strategy**: Gracefully skip unknown blocks during translation
   - Log warnings for visibility but don't fail requests
   - Extract text content where possible

5. **Target Format (OpenAI Multimodal):**
   ```json
   {
     "role": "user",
     "content": [
       { "type": "text", "text": "..." },
       { "type": "image_url", "image_url": { "url": "data:image/jpeg;base64,..." } }
     ]
   }
   ```

## Verification Steps

After implementation:

1. **Test image upload:**
   ```bash
   curl -X POST http://localhost:6767/v1/messages \
     -H "Content-Type: application/json" \
     -d '{
       "model": "gpt-4o",
       "max_tokens": 1024,
       "messages": [{
         "role": "user",
         "content": [{
           "type": "text",
           "text": "What is in this image?"
         }, {
           "type": "image",
           "source": {
             "type": "base64",
             "media_type": "image/jpeg",
             "data": "..."
           }
         }]
       }]
     }'
   ```

2. **Verify no deserialization errors** - should not see 422 error

3. **Check response format** - should be valid Anthropic response

4. **Run test suite:** `cargo test`

5. **Test with Claude Code** - attempt image upload again

## Risk Mitigation

1. **Backward compatibility:** Existing text/tool blocks continue to work
2. **Graceful degradation:** Unknown blocks are skipped, not fatal errors
3. **Clear error messages:** If vision is not supported, explain why
4. **Comprehensive tests:** Cover all new block types and edge cases
