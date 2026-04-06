use copilot_adapter::anthropic::types::*;
use copilot_adapter::copilot::types::{
    ChatCompletionResponse, Choice, Message, MessageContent, Usage as OpenAIUsage,
};

// ---------------------------------------------------------------------------
// E8-T9: Serialization / deserialization
// ---------------------------------------------------------------------------

#[test]
fn anthropic_request_minimal_deserializes() {
    let json = serde_json::json!({
        "model": "claude-sonnet-4-20250514",
        "max_tokens": 1024,
        "messages": [{"role": "user", "content": "Hello"}]
    });
    let req: AnthropicRequest = serde_json::from_value(json).unwrap();
    assert_eq!(req.model, "claude-sonnet-4-20250514");
    assert_eq!(req.max_tokens, 1024);
    assert_eq!(req.messages.len(), 1);
    assert!(req.system.is_none());
    assert!(req.stream.is_none());
}

#[test]
fn anthropic_request_full_deserializes() {
    let json = serde_json::json!({
        "model": "claude-sonnet-4-20250514",
        "max_tokens": 4096,
        "system": "You are a helpful assistant.",
        "messages": [
            {"role": "user", "content": "Hello"},
            {"role": "assistant", "content": "Hi there!"},
            {"role": "user", "content": "How are you?"}
        ],
        "stream": true,
        "temperature": 0.7,
        "top_p": 0.9,
        "stop_sequences": ["END", "STOP"]
    });
    let req: AnthropicRequest = serde_json::from_value(json).unwrap();
    assert_eq!(req.model, "claude-sonnet-4-20250514");
    assert_eq!(req.max_tokens, 4096);
    assert_eq!(
        req.system.as_ref().map(|s| s.to_text()),
        Some("You are a helpful assistant.".to_string())
    );
    assert_eq!(req.messages.len(), 3);
    assert_eq!(req.stream, Some(true));
    assert_eq!(req.temperature, Some(0.7));
    assert_eq!(req.top_p, Some(0.9));
    assert_eq!(req.stop_sequences.as_ref().unwrap().len(), 2);
}

#[test]
fn anthropic_request_with_tool_choice_is_accepted_and_ignored() {
    let json = serde_json::json!({
        "model": "claude-sonnet-4-20250514",
        "max_tokens": 1024,
        "messages": [{"role": "user", "content": "Hello"}],
        "tools": [{"name": "get_weather", "description": "Get weather", "input_schema": {"type": "object"}}],
        "tool_choice": {"type": "tool", "name": "get_weather"}
    });
    let req: AnthropicRequest = serde_json::from_value(json).unwrap();
    // tool_choice is accepted (not rejected) but will be ignored by the adapter
    assert!(req.tool_choice.is_some());
    assert_eq!(req.tool_choice.as_ref().unwrap()["type"], "tool");
}

#[test]
fn anthropic_request_with_content_blocks_deserializes() {
    let json = serde_json::json!({
        "model": "claude-sonnet-4-20250514",
        "max_tokens": 1024,
        "messages": [{
            "role": "user",
            "content": [
                {"type": "text", "text": "Hello "},
                {"type": "text", "text": "world!"}
            ]
        }]
    });
    let req: AnthropicRequest = serde_json::from_value(json).unwrap();
    assert_eq!(req.messages.len(), 1);
    match &req.messages[0].content {
        ContentBlockInput::Blocks(blocks) => {
            assert_eq!(blocks.len(), 2);
            match &blocks[0] {
                ContentBlock::Text { text, .. } => assert_eq!(text, "Hello "),
                _ => panic!("Expected Text variant"),
            }
        }
        ContentBlockInput::Text(_) => panic!("Expected Blocks variant"),
    }
}

#[test]
fn anthropic_request_roundtrip() {
    let req = AnthropicRequest {
        model: "claude-sonnet-4-20250514".to_string(),
        max_tokens: 1024,
        messages: vec![AnthropicMessage {
            role: "user".to_string(),
            content: ContentBlockInput::Text("Hello".to_string()),
        }],
        system: Some(SystemInput::Text("Be helpful".to_string())),
        stream: Some(false),
        temperature: Some(0.5),
        top_p: None,
        stop_sequences: None,
        tools: None,
        tool_choice: None,
        output_config: None,
        thinking: None,
    };
    let json_str = serde_json::to_string(&req).unwrap();
    let deserialized: AnthropicRequest = serde_json::from_str(&json_str).unwrap();
    assert_eq!(deserialized.model, "claude-sonnet-4-20250514");
    assert_eq!(
        deserialized.system.as_ref().map(|s| s.to_text()),
        Some("Be helpful".to_string())
    );
}

#[test]
fn anthropic_response_serializes_correctly() {
    let resp = AnthropicResponse {
        id: "msg_abc123".to_string(),
        response_type: "message".to_string(),
        role: "assistant".to_string(),
        content: vec![ResponseContentBlock::text("Hello!".to_string())],
        model: "claude-sonnet-4-20250514".to_string(),
        stop_reason: Some("end_turn".to_string()),
        stop_sequence: None,
        usage: AnthropicUsage {
            input_tokens: 10,
            output_tokens: 5,
        },
    };
    let json = serde_json::to_value(&resp).unwrap();
    assert_eq!(json["id"], "msg_abc123");
    assert_eq!(json["type"], "message");
    assert_eq!(json["role"], "assistant");
    assert_eq!(json["content"][0]["type"], "text");
    assert_eq!(json["content"][0]["text"], "Hello!");
    assert_eq!(json["stop_reason"], "end_turn");
    assert!(json.get("stop_sequence").is_none());
    assert_eq!(json["usage"]["input_tokens"], 10);
    assert_eq!(json["usage"]["output_tokens"], 5);
}

#[test]
fn anthropic_response_roundtrip() {
    let resp = AnthropicResponse {
        id: "msg_test".to_string(),
        response_type: "message".to_string(),
        role: "assistant".to_string(),
        content: vec![ResponseContentBlock::text("Hi".to_string())],
        model: "claude-sonnet-4-20250514".to_string(),
        stop_reason: Some("end_turn".to_string()),
        stop_sequence: None,
        usage: AnthropicUsage {
            input_tokens: 5,
            output_tokens: 1,
        },
    };
    let json_str = serde_json::to_string(&resp).unwrap();
    let deserialized: AnthropicResponse = serde_json::from_str(&json_str).unwrap();
    assert_eq!(deserialized.id, "msg_test");
    assert_eq!(deserialized.content[0].text_content(), "Hi");
}

#[test]
fn stream_event_message_start_serializes() {
    let event = StreamEvent::MessageStart {
        message: AnthropicResponse {
            id: "msg_123".to_string(),
            response_type: "message".to_string(),
            role: "assistant".to_string(),
            content: vec![],
            model: "claude-sonnet-4-20250514".to_string(),
            stop_reason: None,
            stop_sequence: None,
            usage: AnthropicUsage {
                input_tokens: 0,
                output_tokens: 0,
            },
        },
    };
    let json = serde_json::to_value(&event).unwrap();
    assert_eq!(json["type"], "message_start");
    assert!(json.get("message").is_some());
}

#[test]
fn stream_event_content_block_delta_serializes() {
    let event = StreamEvent::ContentBlockDelta {
        index: 0,
        delta: ContentDelta::Text(TextDelta {
            delta_type: "text_delta".to_string(),
            text: "Hello".to_string(),
        }),
    };
    let json = serde_json::to_value(&event).unwrap();
    assert_eq!(json["type"], "content_block_delta");
    assert_eq!(json["index"], 0);
    assert_eq!(json["delta"]["type"], "text_delta");
    assert_eq!(json["delta"]["text"], "Hello");
}

#[test]
fn stream_event_message_stop_serializes() {
    let event = StreamEvent::MessageStop {};
    let json = serde_json::to_value(&event).unwrap();
    assert_eq!(json["type"], "message_stop");
}

// ---------------------------------------------------------------------------
// E8-T10: Request translation
// ---------------------------------------------------------------------------

#[test]
fn request_translation_with_system_prepends_system_message() {
    let req = AnthropicRequest {
        model: "claude-sonnet-4-20250514".to_string(),
        max_tokens: 1024,
        messages: vec![AnthropicMessage {
            role: "user".to_string(),
            content: ContentBlockInput::Text("Hello".to_string()),
        }],
        system: Some(SystemInput::Text(
            "You are a helpful assistant.".to_string(),
        )),
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
    assert_eq!(openai.messages[0].role, "system");
    assert_eq!(
        openai.messages[0].content.as_text(),
        "You are a helpful assistant."
    );
    assert_eq!(openai.messages[1].role, "user");
    assert_eq!(openai.messages[1].content.as_text(), "Hello");
}

#[test]
fn request_translation_without_system_no_extra_message() {
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
    assert_eq!(openai.messages.len(), 1);
    assert_eq!(openai.messages[0].role, "user");
}

#[test]
fn request_translation_extracts_text_from_content_blocks() {
    let req = AnthropicRequest {
        model: "claude-sonnet-4-20250514".to_string(),
        max_tokens: 1024,
        messages: vec![AnthropicMessage {
            role: "user".to_string(),
            content: ContentBlockInput::Blocks(vec![
                ContentBlock::Text {
                    text: "Hello ".to_string(),
                    cache_control: None,
                },
                ContentBlock::Text {
                    text: "world!".to_string(),
                    cache_control: None,
                },
            ]),
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
    assert_eq!(openai.messages[0].content.as_text(), "Hello \n\nworld!");
}

#[test]
fn request_translation_maps_fields() {
    let req = AnthropicRequest {
        model: "claude-sonnet-4-20250514".to_string(),
        max_tokens: 2048,
        messages: vec![AnthropicMessage {
            role: "user".to_string(),
            content: ContentBlockInput::Text("Hi".to_string()),
        }],
        system: None,
        stream: Some(true),
        temperature: Some(0.7),
        top_p: Some(0.9),
        stop_sequences: Some(vec!["END".to_string()]),
        tools: None,
        tool_choice: None,
        output_config: None,
        thinking: None,
    };

    let openai = req.to_chat_completion_request(false);
    assert_eq!(openai.model, "claude-sonnet-4-20250514");
    assert_eq!(openai.max_tokens, Some(2048));
    assert_eq!(openai.stream, Some(true));
    assert_eq!(openai.temperature, Some(0.7));
    assert_eq!(openai.top_p, Some(0.9));
    assert_eq!(openai.stop, Some(serde_json::json!(["END"])));
}

#[test]
fn request_translation_multiple_messages() {
    let req = AnthropicRequest {
        model: "claude-sonnet-4-20250514".to_string(),
        max_tokens: 1024,
        messages: vec![
            AnthropicMessage {
                role: "user".to_string(),
                content: ContentBlockInput::Text("Hello".to_string()),
            },
            AnthropicMessage {
                role: "assistant".to_string(),
                content: ContentBlockInput::Text("Hi there!".to_string()),
            },
            AnthropicMessage {
                role: "user".to_string(),
                content: ContentBlockInput::Text("How are you?".to_string()),
            },
        ],
        system: Some(SystemInput::Text("Be concise.".to_string())),
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
    assert_eq!(openai.messages.len(), 4);
    assert_eq!(openai.messages[0].role, "system");
    assert_eq!(openai.messages[0].content.as_text(), "Be concise.");
    assert_eq!(openai.messages[1].role, "user");
    assert_eq!(openai.messages[2].role, "assistant");
    assert_eq!(openai.messages[3].role, "user");
}

// ---------------------------------------------------------------------------
// E8-T11: Response translation
// ---------------------------------------------------------------------------

#[test]
fn response_translation_maps_stop_reason_stop() {
    let openai_resp = ChatCompletionResponse {
        id: "chatcmpl-abc123".to_string(),
        object: "chat.completion".to_string(),
        created: 1700000000,
        model: "gpt-4".to_string(),
        choices: vec![Choice {
            index: 0,
            message: Message {
                role: "assistant".to_string(),
                content: MessageContent::Text("Hello!".to_string()),
                name: None,
                tool_calls: None,
                tool_call_id: None,
            },
            finish_reason: Some("stop".to_string()),
        }],
        usage: Some(OpenAIUsage {
            prompt_tokens: 10,
            completion_tokens: 5,
            total_tokens: 15,
            ..Default::default()
        }),
    };

    let anthropic = openai_resp.to_anthropic_response();
    assert_eq!(anthropic.stop_reason, Some("end_turn".to_string()));
}

#[test]
fn response_translation_maps_stop_reason_length() {
    let openai_resp = ChatCompletionResponse {
        id: "chatcmpl-xyz".to_string(),
        object: "chat.completion".to_string(),
        created: 1700000000,
        model: "gpt-4".to_string(),
        choices: vec![Choice {
            index: 0,
            message: Message {
                role: "assistant".to_string(),
                content: MessageContent::Text("Truncated...".to_string()),
                name: None,
                tool_calls: None,
                tool_call_id: None,
            },
            finish_reason: Some("length".to_string()),
        }],
        usage: Some(OpenAIUsage {
            prompt_tokens: 100,
            completion_tokens: 4096,
            total_tokens: 4196,
            ..Default::default()
        }),
    };

    let anthropic = openai_resp.to_anthropic_response();
    assert_eq!(anthropic.stop_reason, Some("max_tokens".to_string()));
}

#[test]
fn response_translation_wraps_content_in_blocks() {
    let openai_resp = ChatCompletionResponse {
        id: "chatcmpl-test".to_string(),
        object: "chat.completion".to_string(),
        created: 1700000000,
        model: "gpt-4".to_string(),
        choices: vec![Choice {
            index: 0,
            message: Message {
                role: "assistant".to_string(),
                content: MessageContent::Text("Hello from Copilot!".to_string()),
                name: None,
                tool_calls: None,
                tool_call_id: None,
            },
            finish_reason: Some("stop".to_string()),
        }],
        usage: Some(OpenAIUsage {
            prompt_tokens: 10,
            completion_tokens: 5,
            total_tokens: 15,
            ..Default::default()
        }),
    };

    let anthropic = openai_resp.to_anthropic_response();
    assert_eq!(anthropic.response_type, "message");
    assert_eq!(anthropic.role, "assistant");
    assert_eq!(anthropic.content.len(), 1);
    assert_eq!(anthropic.content[0].block_type(), "text");
    assert_eq!(anthropic.content[0].text_content(), "Hello from Copilot!");
}

#[test]
fn response_translation_maps_usage() {
    let openai_resp = ChatCompletionResponse {
        id: "chatcmpl-usage".to_string(),
        object: "chat.completion".to_string(),
        created: 1700000000,
        model: "gpt-4".to_string(),
        choices: vec![Choice {
            index: 0,
            message: Message {
                role: "assistant".to_string(),
                content: MessageContent::Text("Hi".to_string()),
                name: None,
                tool_calls: None,
                tool_call_id: None,
            },
            finish_reason: Some("stop".to_string()),
        }],
        usage: Some(OpenAIUsage {
            prompt_tokens: 42,
            completion_tokens: 15,
            total_tokens: 57,
            ..Default::default()
        }),
    };

    let anthropic = openai_resp.to_anthropic_response();
    assert_eq!(anthropic.usage.input_tokens, 42);
    assert_eq!(anthropic.usage.output_tokens, 15);
}

#[test]
fn response_translation_id_format() {
    let openai_resp = ChatCompletionResponse {
        id: "chatcmpl-abc123".to_string(),
        object: "chat.completion".to_string(),
        created: 1700000000,
        model: "gpt-4".to_string(),
        choices: vec![Choice {
            index: 0,
            message: Message {
                role: "assistant".to_string(),
                content: MessageContent::Text("Hi".to_string()),
                name: None,
                tool_calls: None,
                tool_call_id: None,
            },
            finish_reason: Some("stop".to_string()),
        }],
        usage: Some(OpenAIUsage {
            prompt_tokens: 5,
            completion_tokens: 1,
            total_tokens: 6,
            ..Default::default()
        }),
    };

    let anthropic = openai_resp.to_anthropic_response();
    assert_eq!(anthropic.id, "msg_abc123");
}

#[test]
fn response_translation_no_usage_defaults_to_zero() {
    let openai_resp = ChatCompletionResponse {
        id: "chatcmpl-nousage".to_string(),
        object: "chat.completion".to_string(),
        created: 1700000000,
        model: "gpt-4".to_string(),
        choices: vec![Choice {
            index: 0,
            message: Message {
                role: "assistant".to_string(),
                content: MessageContent::Text("Hello".to_string()),
                name: None,
                tool_calls: None,
                tool_call_id: None,
            },
            finish_reason: Some("stop".to_string()),
        }],
        usage: None,
    };

    let anthropic = openai_resp.to_anthropic_response();
    assert_eq!(anthropic.usage.input_tokens, 0);
    assert_eq!(anthropic.usage.output_tokens, 0);
}

#[test]
fn response_translation_empty_choices_returns_empty_content() {
    let openai_resp = ChatCompletionResponse {
        id: "chatcmpl-empty".to_string(),
        object: "chat.completion".to_string(),
        created: 1700000000,
        model: "gpt-4".to_string(),
        choices: vec![],
        usage: None,
    };

    let anthropic = openai_resp.to_anthropic_response();
    // Per Anthropic spec, empty choices → empty content array
    assert!(anthropic.content.is_empty());
    assert!(anthropic.stop_reason.is_none());
}

#[test]
fn map_stop_reason_values() {
    assert_eq!(map_stop_reason(Some("stop")), Some("end_turn".to_string()));
    assert_eq!(
        map_stop_reason(Some("length")),
        Some("max_tokens".to_string())
    );
    assert_eq!(
        map_stop_reason(Some("content_filter")),
        Some("content_filter".to_string())
    );
    assert_eq!(map_stop_reason(None), None);
}

// ---------------------------------------------------------------------------
// Native tools: assistant message with tool_use blocks
// ---------------------------------------------------------------------------

#[test]
fn native_tools_assistant_message_with_tool_use_gets_tool_calls() {
    // When native_tools=true, assistant messages with tool_use blocks should
    // be translated with proper OpenAI tool_calls format.
    let req = AnthropicRequest {
        model: "claude-3".to_string(),
        max_tokens: 1024,
        messages: vec![
            AnthropicMessage {
                role: "user".to_string(),
                content: ContentBlockInput::Text("Search for files".to_string()),
            },
            AnthropicMessage {
                role: "assistant".to_string(),
                content: ContentBlockInput::Blocks(vec![
                    ContentBlock::Text {
                        text: "I'll search for that.".to_string(),
                        cache_control: None,
                    },
                    ContentBlock::ToolUse {
                        id: "toolu_01ABC".to_string(),
                        name: "search_files".to_string(),
                        input: serde_json::json!({"query": "test"}),
                        cache_control: None,
                    },
                ]),
            },
            AnthropicMessage {
                role: "user".to_string(),
                content: ContentBlockInput::Blocks(vec![ContentBlock::ToolResult {
                    tool_use_id: "toolu_01ABC".to_string(),
                    content: ToolResultContent::Text("Found 3 files".to_string()),
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

    // With native_tools=true
    let openai = req.to_chat_completion_request(true);

    // Should have: user, assistant (with tool_calls), tool
    assert_eq!(openai.messages.len(), 3);
    assert_eq!(openai.messages[0].role, "user");
    assert_eq!(openai.messages[1].role, "assistant");
    assert_eq!(openai.messages[2].role, "tool");

    // Assistant message should have tool_calls
    let tool_calls = openai.messages[1].tool_calls.as_ref().unwrap();
    assert_eq!(tool_calls.len(), 1);
    assert_eq!(tool_calls[0].id, Some("toolu_01ABC".to_string()));
    assert_eq!(
        tool_calls[0].function.name,
        Some("search_files".to_string())
    );
    assert_eq!(
        tool_calls[0].function.arguments,
        Some(r#"{"query":"test"}"#.to_string())
    );

    // Tool message should reference the same ID
    assert_eq!(
        openai.messages[2].tool_call_id,
        Some("toolu_01ABC".to_string())
    );
}

#[test]
fn non_native_tools_assistant_message_with_tool_use_no_tool_calls() {
    // When native_tools=false (XML injection mode), assistant messages with
    // tool_use blocks should NOT have tool_calls (the tool_use is embedded in text).
    let req = AnthropicRequest {
        model: "claude-3".to_string(),
        max_tokens: 1024,
        messages: vec![
            AnthropicMessage {
                role: "user".to_string(),
                content: ContentBlockInput::Text("Search for files".to_string()),
            },
            AnthropicMessage {
                role: "assistant".to_string(),
                content: ContentBlockInput::Blocks(vec![
                    ContentBlock::Text {
                        text: "I'll search for that.".to_string(),
                        cache_control: None,
                    },
                    ContentBlock::ToolUse {
                        id: "toolu_01ABC".to_string(),
                        name: "search_files".to_string(),
                        input: serde_json::json!({"query": "test"}),
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

    // With native_tools=false
    let openai = req.to_chat_completion_request(false);

    // Assistant message should NOT have tool_calls
    assert!(openai.messages[1].tool_calls.is_none());
    // Should only have text content
    assert_eq!(
        openai.messages[1].content.as_text(),
        "I'll search for that."
    );
}

#[test]
fn native_tools_multiple_tool_use_blocks() {
    // Test with multiple tool_use blocks in one assistant message
    let req = AnthropicRequest {
        model: "claude-3".to_string(),
        max_tokens: 1024,
        messages: vec![
            AnthropicMessage {
                role: "user".to_string(),
                content: ContentBlockInput::Text("Search and read".to_string()),
            },
            AnthropicMessage {
                role: "assistant".to_string(),
                content: ContentBlockInput::Blocks(vec![
                    ContentBlock::Text {
                        text: "I'll do both.".to_string(),
                        cache_control: None,
                    },
                    ContentBlock::ToolUse {
                        id: "toolu_01ABC".to_string(),
                        name: "search_files".to_string(),
                        input: serde_json::json!({"query": "test"}),
                        cache_control: None,
                    },
                    ContentBlock::ToolUse {
                        id: "toolu_02XYZ".to_string(),
                        name: "read_file".to_string(),
                        input: serde_json::json!({"path": "/tmp/file.txt"}),
                        cache_control: None,
                    },
                ]),
            },
            AnthropicMessage {
                role: "user".to_string(),
                content: ContentBlockInput::Blocks(vec![
                    ContentBlock::ToolResult {
                        tool_use_id: "toolu_01ABC".to_string(),
                        content: ToolResultContent::Text("Found files".to_string()),
                        cache_control: None,
                    },
                    ContentBlock::ToolResult {
                        tool_use_id: "toolu_02XYZ".to_string(),
                        content: ToolResultContent::Text("File contents".to_string()),
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

    let openai = req.to_chat_completion_request(true);

    // Should have: user, assistant (with 2 tool_calls), tool, tool
    assert_eq!(openai.messages.len(), 4);

    // Assistant message should have 2 tool_calls
    let tool_calls = openai.messages[1].tool_calls.as_ref().unwrap();
    assert_eq!(tool_calls.len(), 2);
    assert_eq!(tool_calls[0].id, Some("toolu_01ABC".to_string()));
    assert_eq!(tool_calls[1].id, Some("toolu_02XYZ".to_string()));

    // Both tool messages should have correct IDs
    assert_eq!(
        openai.messages[2].tool_call_id,
        Some("toolu_01ABC".to_string())
    );
    assert_eq!(
        openai.messages[3].tool_call_id,
        Some("toolu_02XYZ".to_string())
    );
}

// ---------------------------------------------------------------------------
// Epic 5 Task 5.5: Effort translation and thinking block tests
// ---------------------------------------------------------------------------

/// Helper to create a request with optional effort and optional thinking.
fn make_request_with_effort_and_thinking(
    effort: Option<&str>,
    thinking: Option<serde_json::Value>,
) -> AnthropicRequest {
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
        output_config: effort.map(|e| OutputConfig {
            effort: Some(e.to_string()),
        }),
        thinking,
    }
}

/// Helper to create a request with thinking and temperature.
fn make_request_with_thinking_and_temp(
    thinking: Option<serde_json::Value>,
    temperature: Option<f64>,
) -> AnthropicRequest {
    AnthropicRequest {
        model: "claude-sonnet-4-20250514".to_string(),
        max_tokens: 1024,
        messages: vec![AnthropicMessage {
            role: "user".to_string(),
            content: ContentBlockInput::Text("Hello".to_string()),
        }],
        system: None,
        stream: None,
        temperature,
        top_p: None,
        stop_sequences: None,
        tools: None,
        tool_choice: None,
        output_config: None,
        thinking,
    }
}

/// Helper to create a request with thinking blocks in conversation history.
fn make_request_with_thinking_blocks_in_history() -> AnthropicRequest {
    AnthropicRequest {
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
    }
}

#[test]
fn effort_low_translates_to_reasoning_low() {
    let request = make_request_with_effort_and_thinking(Some("low"), None);
    let chat_req = request.to_chat_completion_request(false);
    assert_eq!(
        chat_req.reasoning.unwrap().effort.unwrap(),
        "low"
    );
}

#[test]
fn effort_max_translates_to_reasoning_high() {
    let request = make_request_with_effort_and_thinking(Some("max"), None);
    let chat_req = request.to_chat_completion_request(false);
    assert_eq!(
        chat_req.reasoning.unwrap().effort.unwrap(),
        "high"
    );
}

#[test]
fn no_effort_produces_no_reasoning() {
    let request = make_request_with_effort_and_thinking(None, None);
    let chat_req = request.to_chat_completion_request(false);
    assert!(chat_req.reasoning.is_none());
}

#[test]
fn thinking_present_suppresses_temperature() {
    let request = make_request_with_thinking_and_temp(
        Some(serde_json::json!({"type": "adaptive"})),
        Some(0.7),
    );
    let chat_req = request.to_chat_completion_request(false);
    assert!(chat_req.temperature.is_none());
}

#[test]
fn thinking_absent_preserves_temperature() {
    let request = make_request_with_thinking_and_temp(None, Some(0.7));
    let chat_req = request.to_chat_completion_request(false);
    assert_eq!(chat_req.temperature, Some(0.7));
}

#[test]
fn thinking_content_block_deserializes_epic5() {
    let json = serde_json::json!({"type": "thinking", "thinking": "analysis"});
    let block: ContentBlock = serde_json::from_value(json).unwrap();
    assert!(matches!(block, ContentBlock::Thinking { .. }));
}

#[test]
fn redacted_thinking_content_block_deserializes_epic5() {
    let json = serde_json::json!({"type": "redacted_thinking", "data": "base64data"});
    let block: ContentBlock = serde_json::from_value(json).unwrap();
    assert!(matches!(block, ContentBlock::RedactedThinking { .. }));
}

#[test]
fn thinking_blocks_stripped_from_messages() {
    let request = make_request_with_thinking_blocks_in_history();
    let chat_req = request.to_chat_completion_request(false);
    let assistant_msg = chat_req
        .messages
        .iter()
        .find(|m| m.role == "assistant")
        .unwrap();
    match &assistant_msg.content {
        MessageContent::Text(t) => {
            // Assert exact expected text so any leaked thinking content
            // ("Simple arithmetic: 2+2=4") would cause a failure.
            assert_eq!(t, "The answer is 4.");
        }
        _ => panic!("Expected text content"),
    }
}

#[test]
fn request_with_output_config_deserializes() {
    let json = serde_json::json!({
        "model": "claude-opus-4-6",
        "max_tokens": 8192,
        "messages": [],
        "output_config": {"effort": "high"},
        "thinking": {"type": "adaptive"}
    });
    let request: AnthropicRequest = serde_json::from_value(json).unwrap();
    assert_eq!(
        request.output_config.unwrap().effort.unwrap(),
        "high"
    );
    assert!(request.thinking.is_some());
}

// ---------------------------------------------------------------------------
// SystemInput::to_text() — separator tests (Epic 3, LOG-ANALYSIS-FIXES)
// ---------------------------------------------------------------------------

#[test]
fn system_input_single_block_no_trailing_separator() {
    let input = SystemInput::Blocks(vec![ContentBlock::Text {
        text: "Hello world".to_string(),
        cache_control: None,
    }]);
    assert_eq!(input.to_text(), "Hello world");
}

#[test]
fn system_input_multiple_blocks_joined_with_double_newline() {
    let input = SystemInput::Blocks(vec![
        ContentBlock::Text {
            text: "Block one.".to_string(),
            cache_control: None,
        },
        ContentBlock::Text {
            text: "Block two.".to_string(),
            cache_control: None,
        },
        ContentBlock::Text {
            text: "Block three.".to_string(),
            cache_control: None,
        },
    ]);
    assert_eq!(
        input.to_text(),
        "Block one.\n\nBlock two.\n\nBlock three."
    );
}

#[test]
fn system_input_filters_non_text_blocks() {
    let input = SystemInput::Blocks(vec![
        ContentBlock::Text {
            text: "Text block.".to_string(),
            cache_control: None,
        },
        ContentBlock::Image {
            source: ImageSource::Base64 {
                media_type: "image/png".to_string(),
                data: "abc".to_string(),
            },
            cache_control: None,
        },
        ContentBlock::Text {
            text: "Another block.".to_string(),
            cache_control: None,
        },
    ]);
    // Image block is skipped; adjacent text blocks still get separator
    assert_eq!(input.to_text(), "Text block.\n\nAnother block.");
}

#[test]
fn system_input_text_variant_is_unchanged() {
    let input = SystemInput::Text("Plain string system prompt".to_string());
    assert_eq!(input.to_text(), "Plain string system prompt");
}
