use copilot_adapter::anthropic::types::*;
use copilot_adapter::copilot::types::{
    ChatCompletionResponse, Choice, Message, Usage as OpenAIUsage,
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
    assert_eq!(req.system, Some("You are a helpful assistant.".to_string()));
    assert_eq!(req.messages.len(), 3);
    assert_eq!(req.stream, Some(true));
    assert_eq!(req.temperature, Some(0.7));
    assert_eq!(req.top_p, Some(0.9));
    assert_eq!(req.stop_sequences.as_ref().unwrap().len(), 2);
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
                ContentBlock::Text { text } => assert_eq!(text, "Hello "),
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
        system: Some("Be helpful".to_string()),
        stream: Some(false),
        temperature: Some(0.5),
        top_p: None,
        stop_sequences: None,
    };
    let json_str = serde_json::to_string(&req).unwrap();
    let deserialized: AnthropicRequest = serde_json::from_str(&json_str).unwrap();
    assert_eq!(deserialized.model, "claude-sonnet-4-20250514");
    assert_eq!(deserialized.system, Some("Be helpful".to_string()));
}

#[test]
fn anthropic_response_serializes_correctly() {
    let resp = AnthropicResponse {
        id: "msg_abc123".to_string(),
        response_type: "message".to_string(),
        role: "assistant".to_string(),
        content: vec![ResponseContentBlock {
            block_type: "text".to_string(),
            text: "Hello!".to_string(),
        }],
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
        content: vec![ResponseContentBlock {
            block_type: "text".to_string(),
            text: "Hi".to_string(),
        }],
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
    assert_eq!(deserialized.content[0].text, "Hi");
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
        delta: TextDelta {
            delta_type: "text_delta".to_string(),
            text: "Hello".to_string(),
        },
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
        system: Some("You are a helpful assistant.".to_string()),
        stream: None,
        temperature: None,
        top_p: None,
        stop_sequences: None,
    };

    let openai = req.to_chat_completion_request();
    assert_eq!(openai.messages.len(), 2);
    assert_eq!(openai.messages[0].role, "system");
    assert_eq!(openai.messages[0].content, "You are a helpful assistant.");
    assert_eq!(openai.messages[1].role, "user");
    assert_eq!(openai.messages[1].content, "Hello");
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
    };

    let openai = req.to_chat_completion_request();
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
                },
                ContentBlock::Text {
                    text: "world!".to_string(),
                },
            ]),
        }],
        system: None,
        stream: None,
        temperature: None,
        top_p: None,
        stop_sequences: None,
    };

    let openai = req.to_chat_completion_request();
    assert_eq!(openai.messages[0].content, "Hello world!");
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
    };

    let openai = req.to_chat_completion_request();
    assert_eq!(openai.model, "claude-sonnet-4-20250514");
    assert_eq!(openai.max_tokens, Some(2048));
    assert_eq!(openai.stream, Some(true));
    assert_eq!(openai.temperature, Some(0.7));
    assert_eq!(openai.top_p, Some(0.9));
    assert_eq!(
        openai.stop,
        Some(serde_json::json!(["END"]))
    );
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
        system: Some("Be concise.".to_string()),
        stream: None,
        temperature: None,
        top_p: None,
        stop_sequences: None,
    };

    let openai = req.to_chat_completion_request();
    assert_eq!(openai.messages.len(), 4);
    assert_eq!(openai.messages[0].role, "system");
    assert_eq!(openai.messages[0].content, "Be concise.");
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
                content: "Hello!".to_string(),
                name: None,
            },
            finish_reason: Some("stop".to_string()),
        }],
        usage: Some(OpenAIUsage {
            prompt_tokens: 10,
            completion_tokens: 5,
            total_tokens: 15,
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
                content: "Truncated...".to_string(),
                name: None,
            },
            finish_reason: Some("length".to_string()),
        }],
        usage: Some(OpenAIUsage {
            prompt_tokens: 100,
            completion_tokens: 4096,
            total_tokens: 4196,
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
                content: "Hello from Copilot!".to_string(),
                name: None,
            },
            finish_reason: Some("stop".to_string()),
        }],
        usage: Some(OpenAIUsage {
            prompt_tokens: 10,
            completion_tokens: 5,
            total_tokens: 15,
        }),
    };

    let anthropic = openai_resp.to_anthropic_response();
    assert_eq!(anthropic.response_type, "message");
    assert_eq!(anthropic.role, "assistant");
    assert_eq!(anthropic.content.len(), 1);
    assert_eq!(anthropic.content[0].block_type, "text");
    assert_eq!(anthropic.content[0].text, "Hello from Copilot!");
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
                content: "Hi".to_string(),
                name: None,
            },
            finish_reason: Some("stop".to_string()),
        }],
        usage: Some(OpenAIUsage {
            prompt_tokens: 42,
            completion_tokens: 15,
            total_tokens: 57,
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
                content: "Hi".to_string(),
                name: None,
            },
            finish_reason: Some("stop".to_string()),
        }],
        usage: Some(OpenAIUsage {
            prompt_tokens: 5,
            completion_tokens: 1,
            total_tokens: 6,
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
                content: "Hello".to_string(),
                name: None,
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
