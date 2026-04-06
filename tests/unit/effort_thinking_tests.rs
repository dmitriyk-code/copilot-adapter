//! Tests for Epic 4: Effort and Thinking Support
//!
//! Covers:
//! - OutputConfig and effort field deserialization (Task 4.1)
//! - Thinking/RedactedThinking content block variants (Task 4.2)
//! - Reasoning struct serialization (Task 4.3)
//! - Effort translation, thinking block stripping, temperature suppression (Task 4.4)

use copilot_adapter::anthropic::types::*;
use copilot_adapter::copilot::types::*;

// ===========================================================================
// Task 4.1: OutputConfig and AnthropicRequest fields
// ===========================================================================

#[test]
fn anthropic_request_with_output_config_effort_deserializes() {
    let json = serde_json::json!({
        "model": "claude-sonnet-4-20250514",
        "max_tokens": 1024,
        "messages": [{"role": "user", "content": "Hello"}],
        "output_config": {"effort": "high"}
    });
    let req: AnthropicRequest = serde_json::from_value(json).unwrap();
    assert_eq!(
        req.output_config.as_ref().unwrap().effort.as_deref(),
        Some("high")
    );
}

#[test]
fn anthropic_request_with_output_config_extra_fields_ignored() {
    // Extra fields like format and task_budget should be silently ignored
    let json = serde_json::json!({
        "model": "claude-sonnet-4-20250514",
        "max_tokens": 1024,
        "messages": [{"role": "user", "content": "Hello"}],
        "output_config": {
            "effort": "medium",
            "format": "json",
            "task_budget": 100
        }
    });
    let req: AnthropicRequest = serde_json::from_value(json).unwrap();
    assert_eq!(
        req.output_config.as_ref().unwrap().effort.as_deref(),
        Some("medium")
    );
}

#[test]
fn anthropic_request_without_output_config_backward_compatible() {
    let json = serde_json::json!({
        "model": "claude-sonnet-4-20250514",
        "max_tokens": 1024,
        "messages": [{"role": "user", "content": "Hello"}]
    });
    let req: AnthropicRequest = serde_json::from_value(json).unwrap();
    assert!(req.output_config.is_none());
    assert!(req.thinking.is_none());
}

#[test]
fn anthropic_request_with_thinking_config_deserializes() {
    let json = serde_json::json!({
        "model": "claude-sonnet-4-20250514",
        "max_tokens": 1024,
        "messages": [{"role": "user", "content": "Hello"}],
        "thinking": {
            "type": "enabled",
            "budget_tokens": 10000
        }
    });
    let req: AnthropicRequest = serde_json::from_value(json).unwrap();
    assert!(req.thinking.is_some());
    assert_eq!(req.thinking.as_ref().unwrap()["type"], "enabled");
}

#[test]
fn anthropic_request_with_both_output_config_and_thinking() {
    let json = serde_json::json!({
        "model": "claude-sonnet-4-20250514",
        "max_tokens": 16384,
        "messages": [{"role": "user", "content": "Hello"}],
        "output_config": {"effort": "max"},
        "thinking": {"type": "enabled", "budget_tokens": 10000}
    });
    let req: AnthropicRequest = serde_json::from_value(json).unwrap();
    assert_eq!(
        req.output_config.as_ref().unwrap().effort.as_deref(),
        Some("max")
    );
    assert!(req.thinking.is_some());
}

#[test]
fn output_config_with_no_effort_deserializes() {
    let json = serde_json::json!({
        "model": "claude-sonnet-4-20250514",
        "max_tokens": 1024,
        "messages": [{"role": "user", "content": "Hello"}],
        "output_config": {}
    });
    let req: AnthropicRequest = serde_json::from_value(json).unwrap();
    assert!(req.output_config.is_some());
    assert!(req.output_config.as_ref().unwrap().effort.is_none());
}

// ===========================================================================
// Task 4.2: Thinking and RedactedThinking content block variants
// ===========================================================================

#[test]
fn thinking_content_block_deserializes() {
    let json = serde_json::json!({
        "type": "thinking",
        "thinking": "Let me analyze this step by step..."
    });
    let block: ContentBlock = serde_json::from_value(json).unwrap();
    match &block {
        ContentBlock::Thinking { thinking, signature } => {
            assert_eq!(thinking, "Let me analyze this step by step...");
            assert!(signature.is_none());
        }
        _ => panic!("Expected Thinking variant"),
    }
}

#[test]
fn thinking_content_block_with_signature_deserializes() {
    let json = serde_json::json!({
        "type": "thinking",
        "thinking": "Step 1: Consider the options...",
        "signature": "sig_abc123"
    });
    let block: ContentBlock = serde_json::from_value(json).unwrap();
    match &block {
        ContentBlock::Thinking { thinking, signature } => {
            assert_eq!(thinking, "Step 1: Consider the options...");
            assert_eq!(signature.as_deref(), Some("sig_abc123"));
        }
        _ => panic!("Expected Thinking variant"),
    }
}

#[test]
fn redacted_thinking_content_block_deserializes() {
    let json = serde_json::json!({
        "type": "redacted_thinking",
        "data": "YmFzZTY0ZW5jb2RlZGRhdGE="
    });
    let block: ContentBlock = serde_json::from_value(json).unwrap();
    match &block {
        ContentBlock::RedactedThinking { data } => {
            assert_eq!(data, "YmFzZTY0ZW5jb2RlZGRhdGE=");
        }
        _ => panic!("Expected RedactedThinking variant"),
    }
}

#[test]
fn message_with_thinking_blocks_in_history_deserializes() {
    let json = serde_json::json!({
        "model": "claude-sonnet-4-20250514",
        "max_tokens": 1024,
        "messages": [
            {"role": "user", "content": "What is 2+2?"},
            {
                "role": "assistant",
                "content": [
                    {"type": "thinking", "thinking": "Simple arithmetic: 2+2=4"},
                    {"type": "text", "text": "The answer is 4."}
                ]
            },
            {"role": "user", "content": "And 3+3?"}
        ]
    });
    let req: AnthropicRequest = serde_json::from_value(json).unwrap();
    assert_eq!(req.messages.len(), 3);

    // Verify the assistant message has both thinking and text blocks
    if let ContentBlockInput::Blocks(blocks) = &req.messages[1].content {
        assert_eq!(blocks.len(), 2);
        assert!(matches!(blocks[0], ContentBlock::Thinking { .. }));
        assert!(matches!(blocks[1], ContentBlock::Text { .. }));
    } else {
        panic!("Expected Blocks content for assistant message");
    }
}

#[test]
fn message_with_redacted_thinking_in_history_deserializes() {
    let json = serde_json::json!({
        "model": "claude-sonnet-4-20250514",
        "max_tokens": 1024,
        "messages": [
            {"role": "user", "content": "Sensitive question"},
            {
                "role": "assistant",
                "content": [
                    {"type": "redacted_thinking", "data": "cmVkYWN0ZWQ="},
                    {"type": "text", "text": "Here is my response."}
                ]
            }
        ]
    });
    let req: AnthropicRequest = serde_json::from_value(json).unwrap();
    assert_eq!(req.messages.len(), 2);
}

#[test]
fn existing_content_block_types_unaffected() {
    // Verify text, image, document, tool_use, tool_result still work
    let blocks = vec![
        serde_json::json!({"type": "text", "text": "hello"}),
        serde_json::json!({"type": "image", "source": {"type": "base64", "media_type": "image/png", "data": "abc"}}),
        serde_json::json!({"type": "tool_use", "id": "t1", "name": "bash", "input": {}}),
        serde_json::json!({"type": "tool_result", "tool_use_id": "t1", "content": "ok"}),
    ];
    for block_json in blocks {
        let _block: ContentBlock = serde_json::from_value(block_json).unwrap();
    }
}

// ===========================================================================
// Task 4.3: Reasoning struct and ChatCompletionRequest field
// ===========================================================================

#[test]
fn reasoning_serializes_with_effort() {
    let r = Reasoning {
        effort: Some("high".to_string()),
    };
    let json = serde_json::to_value(&r).unwrap();
    assert_eq!(json["effort"], "high");
}

#[test]
fn reasoning_none_effort_omitted() {
    let r = Reasoning { effort: None };
    let json = serde_json::to_value(&r).unwrap();
    assert!(json.get("effort").is_none());
}

#[test]
fn chat_completion_request_with_reasoning_serializes() {
    let req = ChatCompletionRequest {
        model: "claude-sonnet-4.5".to_string(),
        messages: vec![Message {
            role: "user".to_string(),
            content: MessageContent::Text("Hello".to_string()),
            name: None,
            tool_calls: None,
            tool_call_id: None,
        }],
        stream: None,
        temperature: None,
        max_tokens: None,
        top_p: None,
        n: None,
        stop: None,
        presence_penalty: None,
        frequency_penalty: None,
        tools: None,
        tool_choice: None,
        reasoning: Some(Reasoning {
            effort: Some("high".to_string()),
        }),
    };
    let json = serde_json::to_value(&req).unwrap();
    assert_eq!(json["reasoning"]["effort"], "high");
}

#[test]
fn chat_completion_request_without_reasoning_omits_field() {
    let req = ChatCompletionRequest {
        model: "claude-sonnet-4.5".to_string(),
        messages: vec![],
        stream: None,
        temperature: None,
        max_tokens: None,
        top_p: None,
        n: None,
        stop: None,
        presence_penalty: None,
        frequency_penalty: None,
        tools: None,
        tool_choice: None,
        reasoning: None,
    };
    let json = serde_json::to_value(&req).unwrap();
    assert!(json.get("reasoning").is_none());
}

// ===========================================================================
// Task 4.4: Effort translation in to_chat_completion_request
// ===========================================================================

#[test]
fn effort_low_translates_to_reasoning_low() {
    let req = make_request_with_effort("low");
    let openai = req.to_chat_completion_request(false);
    assert_eq!(
        openai.reasoning.as_ref().unwrap().effort.as_deref(),
        Some("low")
    );
}

#[test]
fn effort_medium_translates_to_reasoning_medium() {
    let req = make_request_with_effort("medium");
    let openai = req.to_chat_completion_request(false);
    assert_eq!(
        openai.reasoning.as_ref().unwrap().effort.as_deref(),
        Some("medium")
    );
}

#[test]
fn effort_high_translates_to_reasoning_high() {
    let req = make_request_with_effort("high");
    let openai = req.to_chat_completion_request(false);
    assert_eq!(
        openai.reasoning.as_ref().unwrap().effort.as_deref(),
        Some("high")
    );
}

#[test]
fn effort_max_downgrades_to_reasoning_high() {
    let req = make_request_with_effort("max");
    let openai = req.to_chat_completion_request(false);
    assert_eq!(
        openai.reasoning.as_ref().unwrap().effort.as_deref(),
        Some("high")
    );
}

#[test]
fn no_output_config_produces_no_reasoning() {
    let req = AnthropicRequest {
        model: "claude-sonnet-4-20250514".to_string(),
        max_tokens: 1024,
        messages: vec![AnthropicMessage {
            role: "user".to_string(),
            content: ContentBlockInput::Text("Hello".to_string()),
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
    let openai = req.to_chat_completion_request(false);
    assert!(openai.reasoning.is_none());
}

#[test]
fn output_config_without_effort_produces_no_reasoning() {
    let req = AnthropicRequest {
        model: "claude-sonnet-4-20250514".to_string(),
        max_tokens: 1024,
        messages: vec![AnthropicMessage {
            role: "user".to_string(),
            content: ContentBlockInput::Text("Hello".to_string()),
        }],
        system: None,
        stream: None,
        temperature: None,
        top_p: None,
        stop_sequences: None,
        tools: None,
        tool_choice: None,
        output_config: Some(OutputConfig { effort: None }),
        thinking: None,
    };
    let openai = req.to_chat_completion_request(false);
    assert!(openai.reasoning.is_none());
}

// ===========================================================================
// Task 4.4: Temperature suppression when thinking is active
// ===========================================================================

#[test]
fn thinking_present_suppresses_temperature() {
    let req = AnthropicRequest {
        model: "claude-sonnet-4-20250514".to_string(),
        max_tokens: 1024,
        messages: vec![AnthropicMessage {
            role: "user".to_string(),
            content: ContentBlockInput::Text("Hello".to_string()),
        }],
        system: None,
        stream: None,
        temperature: Some(0.7),
        top_p: None,
        stop_sequences: None,
        tools: None,
        tool_choice: None,
        output_config: None,
        thinking: Some(serde_json::json!({"type": "enabled", "budget_tokens": 5000})),
    };
    let openai = req.to_chat_completion_request(false);
    assert!(
        openai.temperature.is_none(),
        "Temperature should be suppressed when thinking is present"
    );
}

#[test]
fn thinking_absent_temperature_forwarded() {
    let req = AnthropicRequest {
        model: "claude-sonnet-4-20250514".to_string(),
        max_tokens: 1024,
        messages: vec![AnthropicMessage {
            role: "user".to_string(),
            content: ContentBlockInput::Text("Hello".to_string()),
        }],
        system: None,
        stream: None,
        temperature: Some(0.7),
        top_p: None,
        stop_sequences: None,
        tools: None,
        tool_choice: None,
        output_config: None,
        thinking: None,
    };
    let openai = req.to_chat_completion_request(false);
    assert_eq!(openai.temperature, Some(0.7));
}

// ===========================================================================
// Task 4.4: Thinking block stripping during translation
// ===========================================================================

#[test]
fn thinking_blocks_stripped_from_assistant_messages() {
    let req = AnthropicRequest {
        model: "claude-sonnet-4-20250514".to_string(),
        max_tokens: 1024,
        messages: vec![
            AnthropicMessage {
                role: "user".to_string(),
                content: ContentBlockInput::Text("What is 2+2?".to_string()),
            },
            AnthropicMessage {
                role: "assistant".to_string(),
                content: ContentBlockInput::Blocks(vec![
                    ContentBlock::Thinking {
                        thinking: "Simple arithmetic: 2+2=4".to_string(),
                        signature: None,
                    },
                    ContentBlock::Text {
                        text: "The answer is 4.".to_string(),
                        cache_control: None,
                    },
                ]),
            },
            AnthropicMessage {
                role: "user".to_string(),
                content: ContentBlockInput::Text("Thanks!".to_string()),
            },
        ],
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
    let openai = req.to_chat_completion_request(false);

    // System is not set, so messages[0]=user, messages[1]=assistant, messages[2]=user
    assert_eq!(openai.messages.len(), 3);
    // The assistant message should only contain the text, not the thinking block
    assert_eq!(openai.messages[1].content.as_text(), "The answer is 4.");
}

#[test]
fn redacted_thinking_blocks_stripped_from_assistant_messages() {
    let req = AnthropicRequest {
        model: "claude-sonnet-4-20250514".to_string(),
        max_tokens: 1024,
        messages: vec![
            AnthropicMessage {
                role: "user".to_string(),
                content: ContentBlockInput::Text("Question".to_string()),
            },
            AnthropicMessage {
                role: "assistant".to_string(),
                content: ContentBlockInput::Blocks(vec![
                    ContentBlock::RedactedThinking {
                        data: "cmVkYWN0ZWQ=".to_string(),
                    },
                    ContentBlock::Text {
                        text: "My answer.".to_string(),
                        cache_control: None,
                    },
                ]),
            },
        ],
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
    let openai = req.to_chat_completion_request(false);
    assert_eq!(openai.messages.len(), 2);
    assert_eq!(openai.messages[1].content.as_text(), "My answer.");
}

#[test]
fn only_thinking_blocks_produces_empty_text_message() {
    let req = AnthropicRequest {
        model: "claude-sonnet-4-20250514".to_string(),
        max_tokens: 1024,
        messages: vec![
            AnthropicMessage {
                role: "user".to_string(),
                content: ContentBlockInput::Text("Think about this".to_string()),
            },
            AnthropicMessage {
                role: "assistant".to_string(),
                content: ContentBlockInput::Blocks(vec![
                    ContentBlock::Thinking {
                        thinking: "Deep thoughts...".to_string(),
                        signature: Some("sig".to_string()),
                    },
                    ContentBlock::RedactedThinking {
                        data: "cmVkYWN0ZWQ=".to_string(),
                    },
                ]),
            },
        ],
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
    let openai = req.to_chat_completion_request(false);
    // Message with only thinking blocks should produce empty text
    assert_eq!(openai.messages.len(), 2);
    assert_eq!(openai.messages[1].content.as_text(), "");
}

#[test]
fn thinking_blocks_stripped_before_tool_result_extraction() {
    // If a user message has thinking blocks mixed with tool_result blocks
    // (unlikely but defensive), the thinking blocks should be stripped.
    let req = AnthropicRequest {
        model: "claude-sonnet-4-20250514".to_string(),
        max_tokens: 1024,
        messages: vec![
            AnthropicMessage {
                role: "user".to_string(),
                content: ContentBlockInput::Text("Call tool".to_string()),
            },
            AnthropicMessage {
                role: "assistant".to_string(),
                content: ContentBlockInput::Blocks(vec![
                    ContentBlock::Thinking {
                        thinking: "Let me call a tool".to_string(),
                        signature: None,
                    },
                    ContentBlock::ToolUse {
                        id: "toolu_01".to_string(),
                        name: "bash".to_string(),
                        input: serde_json::json!({"command": "ls"}),
                        cache_control: None,
                    },
                ]),
            },
            AnthropicMessage {
                role: "user".to_string(),
                content: ContentBlockInput::Blocks(vec![ContentBlock::ToolResult {
                    tool_use_id: "toolu_01".to_string(),
                    content: ToolResultContent::Text("file1.txt".to_string()),
                    cache_control: None,
                }]),
            },
        ],
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

    // With native_tools=true, the assistant message should have tool_calls
    // but the thinking block should be stripped
    let openai = req.to_chat_completion_request(true);

    // Find the assistant message
    let assistant_msg = openai
        .messages
        .iter()
        .find(|m| m.role == "assistant")
        .unwrap();
    assert!(
        assistant_msg.tool_calls.is_some(),
        "Assistant should have tool_calls"
    );
    // Text should not include the thinking block content
    assert!(
        !assistant_msg.content.as_text().contains("Let me call a tool"),
        "Thinking text should be stripped"
    );
}

#[test]
fn text_and_tool_use_unaffected_by_thinking_stripping() {
    // Verify that text, image, tool_use, tool_result blocks are not affected
    let req = AnthropicRequest {
        model: "claude-sonnet-4-20250514".to_string(),
        max_tokens: 1024,
        messages: vec![
            AnthropicMessage {
                role: "user".to_string(),
                content: ContentBlockInput::Text("Hello world".to_string()),
            },
            AnthropicMessage {
                role: "assistant".to_string(),
                content: ContentBlockInput::Blocks(vec![ContentBlock::Text {
                    text: "Hi there!".to_string(),
                    cache_control: None,
                }]),
            },
        ],
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
    let openai = req.to_chat_completion_request(false);
    assert_eq!(openai.messages.len(), 2);
    assert_eq!(openai.messages[0].content.as_text(), "Hello world");
    assert_eq!(openai.messages[1].content.as_text(), "Hi there!");
}

// ===========================================================================
// End-to-end: full request with effort, thinking, and thinking blocks
// ===========================================================================

#[test]
fn full_request_with_effort_thinking_and_history() {
    let json = serde_json::json!({
        "model": "claude-sonnet-4-20250514",
        "max_tokens": 16384,
        "messages": [
            {"role": "user", "content": "What is the meaning of life?"},
            {
                "role": "assistant",
                "content": [
                    {"type": "thinking", "thinking": "Deep philosophical question...", "signature": "sig123"},
                    {"type": "text", "text": "The meaning of life is subjective."}
                ]
            },
            {"role": "user", "content": "Tell me more."}
        ],
        "output_config": {"effort": "max"},
        "thinking": {"type": "enabled", "budget_tokens": 10000},
        "temperature": 0.9,
        "stream": true
    });
    let req: AnthropicRequest = serde_json::from_value(json).unwrap();
    let openai = req.to_chat_completion_request(false);

    // Effort: "max" → "high"
    assert_eq!(
        openai.reasoning.as_ref().unwrap().effort.as_deref(),
        Some("high")
    );

    // Temperature suppressed because thinking is present
    assert!(openai.temperature.is_none());

    // Stream preserved
    assert_eq!(openai.stream, Some(true));

    // Thinking blocks stripped from assistant message
    assert_eq!(openai.messages.len(), 3); // user, assistant, user
    assert_eq!(
        openai.messages[1].content.as_text(),
        "The meaning of life is subjective."
    );
    assert!(!openai.messages[1].content.as_text().contains("Deep philosophical"));
}

#[test]
fn effort_translation_from_deserialized_request() {
    // Full round-trip: JSON → AnthropicRequest → ChatCompletionRequest → JSON
    let json = serde_json::json!({
        "model": "claude-sonnet-4-20250514",
        "max_tokens": 4096,
        "messages": [{"role": "user", "content": "Hello"}],
        "output_config": {"effort": "low"}
    });
    let req: AnthropicRequest = serde_json::from_value(json).unwrap();
    let openai = req.to_chat_completion_request(false);
    let openai_json = serde_json::to_value(&openai).unwrap();
    assert_eq!(openai_json["reasoning"]["effort"], "low");
}

// ===========================================================================
// Helpers
// ===========================================================================

fn make_request_with_effort(effort: &str) -> AnthropicRequest {
    AnthropicRequest {
        model: "claude-sonnet-4-20250514".to_string(),
        max_tokens: 1024,
        messages: vec![AnthropicMessage {
            role: "user".to_string(),
            content: ContentBlockInput::Text("Hello".to_string()),
        }],
        system: None,
        stream: None,
        temperature: None,
        top_p: None,
        stop_sequences: None,
        tools: None,
        tool_choice: None,
        output_config: Some(OutputConfig {
            effort: Some(effort.to_string()),
        }),
        thinking: None,
    }
}
