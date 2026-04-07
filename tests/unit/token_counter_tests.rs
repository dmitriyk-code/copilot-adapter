use copilot_adapter::anthropic::types::*;
use copilot_adapter::token_counter::{count_tokens, count_tokens_for_request, count_output_tokens};

// ---------------------------------------------------------------------------
// E3-T10: Unit test: count simple text message
// ---------------------------------------------------------------------------

#[test]
fn count_tokens_simple_text_message() {
    let req = CountTokensRequest {
        model: "claude-sonnet-4-20250514".to_string(),
        messages: vec![AnthropicMessage {
            role: "user".to_string(),
            content: ContentBlockInput::Text("Hello!".to_string()),
        }],
        system: None,
        tools: None,
    };
    let count = count_tokens(&req).unwrap();
    // "Hello!" is ~2 tokens + 4 message overhead = ~6
    assert!(count > 0, "Token count should be positive, got {count}");
    assert!(count < 20, "Simple greeting should be under 20 tokens, got {count}");
}

#[test]
fn count_tokens_longer_text_message() {
    let req = CountTokensRequest {
        model: "claude-sonnet-4-20250514".to_string(),
        messages: vec![AnthropicMessage {
            role: "user".to_string(),
            content: ContentBlockInput::Text(
                "The quick brown fox jumps over the lazy dog. This is a longer sentence with more tokens."
                    .to_string(),
            ),
        }],
        system: None,
        tools: None,
    };
    let count = count_tokens(&req).unwrap();
    // Longer text should produce more tokens than a short greeting
    assert!(count > 10, "Longer text should produce more tokens, got {count}");
    assert!(count < 100, "Still reasonable for one sentence, got {count}");
}

// ---------------------------------------------------------------------------
// E3-T11: Unit test: count with system prompt
// ---------------------------------------------------------------------------

#[test]
fn count_tokens_with_system_string() {
    let without_system = CountTokensRequest {
        model: "claude-sonnet-4-20250514".to_string(),
        messages: vec![AnthropicMessage {
            role: "user".to_string(),
            content: ContentBlockInput::Text("Hello".to_string()),
        }],
        system: None,
        tools: None,
    };

    let with_system = CountTokensRequest {
        model: "claude-sonnet-4-20250514".to_string(),
        messages: vec![AnthropicMessage {
            role: "user".to_string(),
            content: ContentBlockInput::Text("Hello".to_string()),
        }],
        system: Some(SystemInput::Text("You are a helpful assistant.".to_string())),
        tools: None,
    };

    let count_without = count_tokens(&without_system).unwrap();
    let count_with = count_tokens(&with_system).unwrap();

    assert!(
        count_with > count_without,
        "System prompt should increase count: without={count_without}, with={count_with}"
    );
}

#[test]
fn count_tokens_with_system_blocks() {
    let req = CountTokensRequest {
        model: "claude-sonnet-4-20250514".to_string(),
        messages: vec![AnthropicMessage {
            role: "user".to_string(),
            content: ContentBlockInput::Text("Hello".to_string()),
        }],
        system: Some(SystemInput::Blocks(vec![ContentBlock::Text {
            text: "You are a helpful assistant.".to_string(),
            cache_control: None,
        }])),
        tools: None,
    };
    let count = count_tokens(&req).unwrap();
    // System prompt adds tokens on top of the message
    assert!(count > 5, "System block should add tokens, got {count}");
}

// ---------------------------------------------------------------------------
// E3-T12: Unit test: count with tools
// ---------------------------------------------------------------------------

#[test]
fn count_tokens_with_tool_definitions() {
    let without_tools = CountTokensRequest {
        model: "claude-sonnet-4-20250514".to_string(),
        messages: vec![AnthropicMessage {
            role: "user".to_string(),
            content: ContentBlockInput::Text("Search for foo".to_string()),
        }],
        system: None,
        tools: None,
    };

    let with_tools = CountTokensRequest {
        model: "claude-sonnet-4-20250514".to_string(),
        messages: vec![AnthropicMessage {
            role: "user".to_string(),
            content: ContentBlockInput::Text("Search for foo".to_string()),
        }],
        system: None,
        tools: Some(vec![
            ToolDefinition {
                name: "grep".to_string(),
                description: Some("Search files for a pattern".to_string()),
                input_schema: InputSchema {
                    schema_type: "object".to_string(),
                    properties: Some(serde_json::json!({
                        "pattern": {"type": "string"},
                        "path": {"type": "string"}
                    })),
                    required: Some(vec!["pattern".to_string()]),
                },
            },
            ToolDefinition {
                name: "read_file".to_string(),
                description: Some("Read a file by path".to_string()),
                input_schema: InputSchema {
                    schema_type: "object".to_string(),
                    properties: Some(serde_json::json!({
                        "path": {"type": "string"}
                    })),
                    required: Some(vec!["path".to_string()]),
                },
            },
        ]),
    };

    let count_without = count_tokens(&without_tools).unwrap();
    let count_with = count_tokens(&with_tools).unwrap();

    assert!(
        count_with > count_without,
        "Tools should increase token count: without={count_without}, with={count_with}"
    );
    // Two tool definitions with schemas should add a meaningful number of tokens
    assert!(
        count_with - count_without > 20,
        "Two tool definitions should add at least 20 tokens, added {}",
        count_with - count_without
    );
}

// ---------------------------------------------------------------------------
// E3-T13: Unit test: count with multiple messages
// ---------------------------------------------------------------------------

#[test]
fn count_tokens_with_multiple_messages() {
    let single_msg = CountTokensRequest {
        model: "claude-sonnet-4-20250514".to_string(),
        messages: vec![AnthropicMessage {
            role: "user".to_string(),
            content: ContentBlockInput::Text("Hello".to_string()),
        }],
        system: None,
        tools: None,
    };

    let multi_msg = CountTokensRequest {
        model: "claude-sonnet-4-20250514".to_string(),
        messages: vec![
            AnthropicMessage {
                role: "user".to_string(),
                content: ContentBlockInput::Text("Hello".to_string()),
            },
            AnthropicMessage {
                role: "assistant".to_string(),
                content: ContentBlockInput::Text("Hi there! How can I help?".to_string()),
            },
            AnthropicMessage {
                role: "user".to_string(),
                content: ContentBlockInput::Text("Tell me a joke.".to_string()),
            },
        ],
        system: None,
        tools: None,
    };

    let count_single = count_tokens(&single_msg).unwrap();
    let count_multi = count_tokens(&multi_msg).unwrap();

    assert!(
        count_multi > count_single,
        "Multiple messages should have higher count: single={count_single}, multi={count_multi}"
    );
    // Each additional message adds at least 4 overhead tokens
    assert!(
        count_multi >= count_single + 8,
        "Two additional messages should add at least 8 tokens (overhead), got diff={}",
        count_multi - count_single
    );
}

#[test]
fn count_tokens_per_message_overhead() {
    // Verify that empty messages still get the 4-token overhead
    let req = CountTokensRequest {
        model: "claude-sonnet-4-20250514".to_string(),
        messages: vec![
            AnthropicMessage {
                role: "user".to_string(),
                content: ContentBlockInput::Text("".to_string()),
            },
            AnthropicMessage {
                role: "assistant".to_string(),
                content: ContentBlockInput::Text("".to_string()),
            },
        ],
        system: None,
        tools: None,
    };
    let count = count_tokens(&req).unwrap();
    // Two empty messages = 2 * 4 overhead = 8
    assert_eq!(count, 8, "Two empty messages should be 8 (overhead only), got {count}");
}

// ---------------------------------------------------------------------------
// E3-T14: Unit test: image block uses estimate
// ---------------------------------------------------------------------------

#[test]
fn count_tokens_image_block_uses_fixed_estimate() {
    let req = CountTokensRequest {
        model: "claude-sonnet-4-20250514".to_string(),
        messages: vec![AnthropicMessage {
            role: "user".to_string(),
            content: ContentBlockInput::Blocks(vec![
                ContentBlock::Text {
                    text: "What is in this image?".to_string(),
                    cache_control: None,
                },
                ContentBlock::Image {
                    source: ImageSource::Base64 {
                        media_type: "image/png".to_string(),
                        data: "iVBORw0KGgo=".to_string(), // dummy base64
                    },
                    cache_control: None,
                },
            ]),
        }],
        system: None,
        tools: None,
    };
    let count = count_tokens(&req).unwrap();
    // Image block = 85 tokens estimate + text tokens + 4 overhead
    // The total should be at least 85 (image) + 4 (overhead) = 89
    assert!(
        count >= 89,
        "Image block should contribute at least 85 tokens, total should be >= 89, got {count}"
    );
}

#[test]
fn count_tokens_multiple_images() {
    let req = CountTokensRequest {
        model: "claude-sonnet-4-20250514".to_string(),
        messages: vec![AnthropicMessage {
            role: "user".to_string(),
            content: ContentBlockInput::Blocks(vec![
                ContentBlock::Image {
                    source: ImageSource::Base64 {
                        media_type: "image/png".to_string(),
                        data: "abc".to_string(),
                    },
                    cache_control: None,
                },
                ContentBlock::Image {
                    source: ImageSource::Base64 {
                        media_type: "image/jpeg".to_string(),
                        data: "def".to_string(),
                    },
                    cache_control: None,
                },
            ]),
        }],
        system: None,
        tools: None,
    };
    let count = count_tokens(&req).unwrap();
    // 2 images * 85 + 4 overhead = 174
    assert!(
        count >= 174,
        "Two image blocks should contribute at least 170 tokens, got {count}"
    );
}

// ---------------------------------------------------------------------------
// Additional edge case tests
// ---------------------------------------------------------------------------

#[test]
fn count_tokens_with_tool_use_block() {
    let req = CountTokensRequest {
        model: "claude-sonnet-4-20250514".to_string(),
        messages: vec![
            AnthropicMessage {
                role: "user".to_string(),
                content: ContentBlockInput::Text("Search for foo".to_string()),
            },
            AnthropicMessage {
                role: "assistant".to_string(),
                content: ContentBlockInput::Blocks(vec![ContentBlock::ToolUse {
                    id: "tu_123".to_string(),
                    name: "grep".to_string(),
                    input: serde_json::json!({"pattern": "foo"}),
                    cache_control: None,
                }]),
            },
        ],
        system: None,
        tools: None,
    };
    let count = count_tokens(&req).unwrap();
    assert!(count > 8, "Tool use block should add tokens, got {count}");
}

#[test]
fn count_tokens_with_tool_result_text() {
    let req = CountTokensRequest {
        model: "claude-sonnet-4-20250514".to_string(),
        messages: vec![AnthropicMessage {
            role: "user".to_string(),
            content: ContentBlockInput::Blocks(vec![ContentBlock::ToolResult {
                tool_use_id: "tu_123".to_string(),
                content: ToolResultContent::Text("Found foo in bar.txt line 42".to_string()),
                cache_control: None,
            }]),
        }],
        system: None,
        tools: None,
    };
    let count = count_tokens(&req).unwrap();
    assert!(count > 4, "Tool result should add tokens beyond overhead, got {count}");
}

#[test]
fn count_tokens_with_tool_result_blocks() {
    let req = CountTokensRequest {
        model: "claude-sonnet-4-20250514".to_string(),
        messages: vec![AnthropicMessage {
            role: "user".to_string(),
            content: ContentBlockInput::Blocks(vec![ContentBlock::ToolResult {
                tool_use_id: "tu_456".to_string(),
                content: ToolResultContent::Blocks(vec![
                    ContentBlock::Text {
                        text: "Result line 1".to_string(),
                        cache_control: None,
                    },
                    ContentBlock::Text {
                        text: "Result line 2".to_string(),
                        cache_control: None,
                    },
                ]),
                cache_control: None,
            }]),
        }],
        system: None,
        tools: None,
    };
    let count = count_tokens(&req).unwrap();
    assert!(count > 4, "Tool result with blocks should add tokens, got {count}");
}

#[test]
fn count_tokens_full_conversation() {
    let req = CountTokensRequest {
        model: "claude-sonnet-4-20250514".to_string(),
        messages: vec![
            AnthropicMessage {
                role: "user".to_string(),
                content: ContentBlockInput::Text("Search for foo".to_string()),
            },
            AnthropicMessage {
                role: "assistant".to_string(),
                content: ContentBlockInput::Blocks(vec![ContentBlock::ToolUse {
                    id: "tu_1".to_string(),
                    name: "grep".to_string(),
                    input: serde_json::json!({"pattern": "foo"}),
                    cache_control: None,
                }]),
            },
            AnthropicMessage {
                role: "user".to_string(),
                content: ContentBlockInput::Blocks(vec![ContentBlock::ToolResult {
                    tool_use_id: "tu_1".to_string(),
                    content: ToolResultContent::Text("found foo in bar.txt".to_string()),
                    cache_control: None,
                }]),
            },
        ],
        system: Some(SystemInput::Text("You are a coding assistant.".to_string())),
        tools: Some(vec![ToolDefinition {
            name: "grep".to_string(),
            description: Some("Search files".to_string()),
            input_schema: InputSchema {
                schema_type: "object".to_string(),
                properties: Some(serde_json::json!({
                    "pattern": {"type": "string"}
                })),
                required: Some(vec!["pattern".to_string()]),
            },
        }]),
    };
    let count = count_tokens(&req).unwrap();
    // Full conversation with system + tools + messages should have a meaningful count
    assert!(
        count > 30,
        "Full conversation should have a significant token count, got {count}"
    );
}

#[test]
fn count_tokens_empty_messages_list() {
    let req = CountTokensRequest {
        model: "claude-sonnet-4-20250514".to_string(),
        messages: vec![],
        system: None,
        tools: None,
    };
    let count = count_tokens(&req).unwrap();
    assert_eq!(count, 0, "Empty request should return 0 tokens, got {count}");
}

// ---------------------------------------------------------------------------
// Review fix: Document block uses fixed estimate (same as image)
// ---------------------------------------------------------------------------

#[test]
fn count_tokens_document_block_uses_fixed_estimate() {
    let req = CountTokensRequest {
        model: "claude-sonnet-4-20250514".to_string(),
        messages: vec![AnthropicMessage {
            role: "user".to_string(),
            content: ContentBlockInput::Blocks(vec![ContentBlock::Document {
                source: DocumentSource::Base64 {
                    media_type: "application/pdf".to_string(),
                    data: "JVBERi0xLjQ=".to_string(), // dummy base64 PDF header
                },
                title: Some("test.pdf".to_string()),
                cache_control: None,
            }]),
        }],
        system: None,
        tools: None,
    };
    let count = count_tokens(&req).unwrap();
    // Document block = 85 tokens estimate + 4 overhead = exactly 89
    assert_eq!(
        count, 89,
        "Document block (85) + message overhead (4) should be exactly 89 tokens, got {count}"
    );
}

// ---------------------------------------------------------------------------
// Review fix: ToolResult with non-text blocks (exercises serialization fallback)
// ---------------------------------------------------------------------------

#[test]
fn count_tokens_tool_result_with_image_block() {
    let req = CountTokensRequest {
        model: "claude-sonnet-4-20250514".to_string(),
        messages: vec![AnthropicMessage {
            role: "user".to_string(),
            content: ContentBlockInput::Blocks(vec![ContentBlock::ToolResult {
                tool_use_id: "tu_img".to_string(),
                content: ToolResultContent::Blocks(vec![
                    ContentBlock::Text {
                        text: "Here is the screenshot:".to_string(),
                        cache_control: None,
                    },
                    ContentBlock::Image {
                        source: ImageSource::Base64 {
                            media_type: "image/png".to_string(),
                            data: "iVBORw0KGgo=".to_string(),
                        },
                        cache_control: None,
                    },
                ]),
                cache_control: None,
            }]),
        }],
        system: None,
        tools: None,
    };
    let count = count_tokens(&req).unwrap();
    // The Image block inside the ToolResult is serialized to JSON (not using
    // the fixed estimate), so the result includes the text content plus the
    // serialized image JSON (base64 data, media_type, etc. — ~80+ characters).
    // The total must be well above the 4-token per-message overhead.
    assert!(
        count > 20,
        "Tool result with image block JSON serialization should produce substantially more than overhead tokens, got {count}"
    );
}

// ---------------------------------------------------------------------------
// Review fix: TokenCountError formatting
// ---------------------------------------------------------------------------

#[test]
fn token_count_error_serialization_formats_message() {
    let err = copilot_adapter::token_counter::TokenCountError::Serialization(
        "test error message".to_string(),
    );
    let display = format!("{err}");
    assert_eq!(display, "Failed to serialize: test error message");
}

#[test]
fn token_count_error_encoder_init_formats_message() {
    let err = copilot_adapter::token_counter::TokenCountError::EncoderInit(
        "vocab not found".to_string(),
    );
    let display = format!("{err}");
    assert_eq!(display, "Failed to initialize tokenizer: vocab not found");
}

// ---------------------------------------------------------------------------
// Epic 6-T1: count_tokens_for_request & count_output_tokens tests
// ---------------------------------------------------------------------------

#[test]
fn count_tokens_for_request_matches_count_tokens() {
    let text = "Hello, world!";
    let anthropic_req = AnthropicRequest {
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
    };
    let count_req = CountTokensRequest {
        model: "claude-sonnet-4-20250514".to_string(),
        messages: vec![AnthropicMessage {
            role: "user".to_string(),
            content: ContentBlockInput::Text(text.to_string()),
        }],
        system: None,
        tools: None,
    };

    let from_request = count_tokens_for_request(&anthropic_req);
    let from_count = count_tokens(&count_req).unwrap();
    assert_eq!(
        from_request, from_count,
        "count_tokens_for_request should match count_tokens: for_request={from_request}, count={from_count}"
    );
}

#[test]
fn count_output_tokens_empty_string() {
    assert_eq!(
        count_output_tokens(""),
        0,
        "Empty string should produce 0 output tokens"
    );
}

#[test]
fn count_output_tokens_nonempty_string() {
    let count = count_output_tokens("Hello, world!");
    assert!(
        count > 0,
        "Non-empty string should produce > 0 output tokens, got {count}"
    );
}

#[test]
fn count_output_tokens_increases_with_length() {
    let short = count_output_tokens("Hi");
    let long = count_output_tokens(
        "The quick brown fox jumps over the lazy dog. \
         This is a much longer sentence with many more tokens \
         that should produce a significantly higher count.",
    );
    assert!(
        long > short,
        "Longer text should produce more output tokens: short={short}, long={long}"
    );
}
