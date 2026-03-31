use copilot_adapter::copilot::types::*;

#[test]
fn image_url_block_serializes_to_openai_format() {
    let block = ContentBlock::ImageUrl {
        image_url: ImageUrl {
            url: "data:image/jpeg;base64,/9j/4AAQ...".to_string(),
            detail: None,
        },
    };
    let json = serde_json::to_value(&block).unwrap();
    assert_eq!(json["type"], "image_url");
    assert_eq!(
        json["image_url"]["url"],
        "data:image/jpeg;base64,/9j/4AAQ..."
    );
    assert!(json["image_url"].get("detail").is_none());
}

#[test]
fn image_url_block_with_detail_serializes() {
    let block = ContentBlock::ImageUrl {
        image_url: ImageUrl {
            url: "https://example.com/image.png".to_string(),
            detail: Some("high".to_string()),
        },
    };
    let json = serde_json::to_value(&block).unwrap();
    assert_eq!(json["type"], "image_url");
    assert_eq!(json["image_url"]["url"], "https://example.com/image.png");
    assert_eq!(json["image_url"]["detail"], "high");
}

#[test]
fn image_url_block_deserializes_from_openai_format() {
    let json = serde_json::json!({
        "type": "image_url",
        "image_url": {
            "url": "data:image/png;base64,iVBOR..."
        }
    });
    let block: ContentBlock = serde_json::from_value(json).unwrap();
    match &block {
        ContentBlock::ImageUrl { image_url } => {
            assert_eq!(image_url.url, "data:image/png;base64,iVBOR...");
            assert!(image_url.detail.is_none());
        }
        _ => panic!("Expected ImageUrl variant"),
    }
}

#[test]
fn as_text_skips_image_blocks() {
    let content = MessageContent::Blocks(vec![
        ContentBlock::Text {
            text: "What is in this image?".to_string(),
        },
        ContentBlock::ImageUrl {
            image_url: ImageUrl {
                url: "data:image/jpeg;base64,/9j/4AAQ...".to_string(),
                detail: None,
            },
        },
    ]);
    assert_eq!(content.as_text(), "What is in this image?");
}

#[test]
fn as_text_returns_empty_when_only_images() {
    let content = MessageContent::Blocks(vec![ContentBlock::ImageUrl {
        image_url: ImageUrl {
            url: "data:image/jpeg;base64,/9j/...".to_string(),
            detail: Some("low".to_string()),
        },
    }]);
    assert_eq!(content.as_text(), "");
}

#[test]
fn message_with_multimodal_content_roundtrips() {
    let json = serde_json::json!({
        "role": "user",
        "content": [
            {"type": "text", "text": "What is in this image?"},
            {"type": "image_url", "image_url": {"url": "data:image/jpeg;base64,..."}}
        ]
    });
    let msg: Message = serde_json::from_value(json).unwrap();
    assert_eq!(msg.role, "user");
    assert_eq!(msg.content.as_text(), "What is in this image?");

    // Re-serialize and verify structure
    let serialized = serde_json::to_value(&msg).unwrap();
    let blocks = serialized["content"].as_array().unwrap();
    assert_eq!(blocks.len(), 2);
    assert_eq!(blocks[0]["type"], "text");
    assert_eq!(blocks[1]["type"], "image_url");
    assert_eq!(blocks[1]["image_url"]["url"], "data:image/jpeg;base64,...");
}

// ===========================================================================
// Epic 2: Request serialization with native OpenAI tools
// ===========================================================================

#[test]
fn request_with_openai_tools_serializes_correctly() {
    let req = ChatCompletionRequest {
        model: "gpt-4o".to_string(),
        messages: vec![Message {
            role: "user".to_string(),
            content: MessageContent::Text("Read a file".to_string()),
            name: None,
            tool_calls: None,
            tool_call_id: None,
        }],
        stream: Some(true),
        temperature: None,
        max_tokens: None,
        top_p: None,
        n: None,
        stop: None,
        presence_penalty: None,
        frequency_penalty: None,
        tools: Some(vec![
            OpenAITool {
                tool_type: "function".to_string(),
                function: OpenAIToolFunction {
                    name: "read_file".to_string(),
                    description: Some("Read a file from disk".to_string()),
                    parameters: Some(serde_json::json!({
                        "type": "object",
                        "properties": {
                            "path": {"type": "string", "description": "File path"}
                        },
                        "required": ["path"]
                    })),
                },
            },
            OpenAITool {
                tool_type: "function".to_string(),
                function: OpenAIToolFunction {
                    name: "write_file".to_string(),
                    description: Some("Write a file to disk".to_string()),
                    parameters: Some(serde_json::json!({
                        "type": "object",
                        "properties": {
                            "path": {"type": "string"},
                            "content": {"type": "string"}
                        },
                        "required": ["path", "content"]
                    })),
                },
            },
        ]),
        tool_choice: Some(serde_json::json!("auto")),
    };

    let json = serde_json::to_value(&req).unwrap();

    // tools array present with correct structure
    let tools = json["tools"].as_array().unwrap();
    assert_eq!(tools.len(), 2);
    assert_eq!(tools[0]["type"], "function");
    assert_eq!(tools[0]["function"]["name"], "read_file");
    assert_eq!(tools[0]["function"]["description"], "Read a file from disk");
    assert_eq!(tools[0]["function"]["parameters"]["type"], "object");
    assert_eq!(tools[1]["function"]["name"], "write_file");

    // tool_choice present
    assert_eq!(json["tool_choice"], "auto");
}

#[test]
fn request_without_tools_omits_tools_fields() {
    let req = ChatCompletionRequest {
        model: "gpt-4o".to_string(),
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
    };

    let json = serde_json::to_value(&req).unwrap();
    assert!(
        json.get("tools").is_none(),
        "tools should be omitted when None"
    );
    assert!(
        json.get("tool_choice").is_none(),
        "tool_choice should be omitted when None"
    );
}

#[test]
fn request_with_specific_tool_choice_serializes() {
    let req = ChatCompletionRequest {
        model: "gpt-4o".to_string(),
        messages: vec![Message {
            role: "user".to_string(),
            content: MessageContent::Text("Test".to_string()),
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
        tools: Some(vec![OpenAITool {
            tool_type: "function".to_string(),
            function: OpenAIToolFunction {
                name: "bash".to_string(),
                description: None,
                parameters: None,
            },
        }]),
        tool_choice: Some(serde_json::json!({
            "type": "function",
            "function": {"name": "bash"}
        })),
    };

    let json = serde_json::to_value(&req).unwrap();
    assert_eq!(json["tool_choice"]["type"], "function");
    assert_eq!(json["tool_choice"]["function"]["name"], "bash");
}

// ===========================================================================
// Epic 2: Response deserialization with tool_calls
// ===========================================================================

#[test]
fn response_with_null_content_and_tool_calls_deserializes() {
    let json = serde_json::json!({
        "id": "chatcmpl-abc",
        "object": "chat.completion",
        "created": 1700000000,
        "model": "gpt-4o",
        "choices": [{
            "index": 0,
            "message": {
                "role": "assistant",
                "content": null,
                "tool_calls": [{
                    "id": "call_001",
                    "type": "function",
                    "function": {
                        "name": "bash",
                        "arguments": "{\"command\":\"ls -la\"}"
                    }
                }]
            },
            "finish_reason": "tool_calls"
        }],
        "usage": {"prompt_tokens": 10, "completion_tokens": 5, "total_tokens": 15}
    });

    let resp: ChatCompletionResponse = serde_json::from_value(json).unwrap();
    let choice = &resp.choices[0];

    // null content → empty text
    assert_eq!(choice.message.content.as_text(), "");
    assert_eq!(choice.finish_reason, Some("tool_calls".to_string()));

    let tc = &choice.message.tool_calls.as_ref().unwrap()[0];
    assert_eq!(tc.id, Some("call_001".to_string()));
    assert_eq!(tc.call_type, Some("function".to_string()));
    assert_eq!(tc.function.name, Some("bash".to_string()));
    assert_eq!(
        tc.function.arguments,
        Some("{\"command\":\"ls -la\"}".to_string())
    );
}

#[test]
fn null_message_content_serializes_as_empty_string() {
    // After deserializing null → empty text, verify it serializes as ""
    let json = serde_json::json!({
        "role": "assistant",
        "content": null
    });
    let msg: Message = serde_json::from_value(json).unwrap();
    assert_eq!(msg.content.as_text(), "");

    // Re-serialized content should be "" (not null)
    let serialized = serde_json::to_value(&msg).unwrap();
    assert_eq!(serialized["content"], "");
}

#[test]
fn message_content_string_roundtrips() {
    let json = serde_json::json!({
        "role": "user",
        "content": "Hello, world!"
    });
    let msg: Message = serde_json::from_value(json).unwrap();
    assert_eq!(msg.content.as_text(), "Hello, world!");
}

#[test]
fn message_content_default_is_empty_text() {
    let default = MessageContent::default();
    assert_eq!(default.as_text(), "");
}

#[test]
fn message_without_content_field_deserializes_to_empty_text() {
    // When the content field is entirely absent from JSON, serde(default) kicks in.
    let json = serde_json::json!({
        "role": "assistant"
    });
    let msg: Message = serde_json::from_value(json).unwrap();
    assert_eq!(msg.content.as_text(), "");
}

// ===========================================================================
// Epic 2: Streaming chunk deserialization with StreamingToolCall
// ===========================================================================

#[test]
fn streaming_chunk_with_streaming_tool_call_deserializes() {
    let json = serde_json::json!({
        "id": "chatcmpl-stc",
        "object": "chat.completion.chunk",
        "created": 1700000000,
        "model": "gpt-4o",
        "choices": [{
            "index": 0,
            "delta": {
                "tool_calls": [{
                    "index": 0,
                    "id": "call_stc_001",
                    "type": "function",
                    "function": {
                        "name": "bash",
                        "arguments": ""
                    }
                }]
            },
            "finish_reason": null
        }]
    });

    let chunk: ChatCompletionChunk = serde_json::from_value(json).unwrap();
    let tc = &chunk.choices[0].delta.tool_calls.as_ref().unwrap()[0];
    assert_eq!(tc.index, 0);
    assert_eq!(tc.id, Some("call_stc_001".to_string()));
    assert_eq!(tc.call_type, Some("function".to_string()));
    let func = tc.function.as_ref().unwrap();
    assert_eq!(func.name, Some("bash".to_string()));
    assert_eq!(func.arguments, Some("".to_string()));
}

#[test]
fn streaming_chunk_partial_arguments_only() {
    let json = serde_json::json!({
        "id": "chatcmpl-stc",
        "object": "chat.completion.chunk",
        "created": 1700000000,
        "model": "gpt-4o",
        "choices": [{
            "index": 0,
            "delta": {
                "tool_calls": [{
                    "index": 0,
                    "function": {
                        "arguments": "{\"path\":\"/tmp"
                    }
                }]
            },
            "finish_reason": null
        }]
    });

    let chunk: ChatCompletionChunk = serde_json::from_value(json).unwrap();
    let tc = &chunk.choices[0].delta.tool_calls.as_ref().unwrap()[0];
    assert_eq!(tc.index, 0);
    assert!(tc.id.is_none());
    assert!(tc.call_type.is_none());
    let func = tc.function.as_ref().unwrap();
    assert!(func.name.is_none());
    assert_eq!(func.arguments, Some("{\"path\":\"/tmp".to_string()));
}

#[test]
fn streaming_chunk_parallel_tool_calls_deserializes() {
    // Two parallel tool calls in a single chunk (different indices).
    let json = serde_json::json!({
        "id": "chatcmpl-parallel",
        "object": "chat.completion.chunk",
        "created": 1700000000,
        "model": "gpt-4o",
        "choices": [{
            "index": 0,
            "delta": {
                "tool_calls": [
                    {
                        "index": 0,
                        "id": "call_p1",
                        "type": "function",
                        "function": {"name": "bash", "arguments": ""}
                    },
                    {
                        "index": 1,
                        "id": "call_p2",
                        "type": "function",
                        "function": {"name": "read_file", "arguments": ""}
                    }
                ]
            },
            "finish_reason": null
        }]
    });

    let chunk: ChatCompletionChunk = serde_json::from_value(json).unwrap();
    let tool_calls = chunk.choices[0].delta.tool_calls.as_ref().unwrap();
    assert_eq!(tool_calls.len(), 2);
    assert_eq!(tool_calls[0].index, 0);
    assert_eq!(tool_calls[0].id, Some("call_p1".to_string()));
    assert_eq!(
        tool_calls[0].function.as_ref().unwrap().name,
        Some("bash".to_string())
    );
    assert_eq!(tool_calls[1].index, 1);
    assert_eq!(tool_calls[1].id, Some("call_p2".to_string()));
    assert_eq!(
        tool_calls[1].function.as_ref().unwrap().name,
        Some("read_file".to_string())
    );
}

#[test]
fn streaming_tool_call_serializes_correctly() {
    let tc = StreamingToolCall {
        index: 0,
        id: Some("call_ser".to_string()),
        call_type: Some("function".to_string()),
        function: Some(StreamingFunctionCall {
            name: Some("bash".to_string()),
            arguments: Some("{\"cmd\":\"ls\"}".to_string()),
        }),
    };

    let json = serde_json::to_value(&tc).unwrap();
    assert_eq!(json["index"], 0);
    assert_eq!(json["id"], "call_ser");
    assert_eq!(json["type"], "function");
    assert_eq!(json["function"]["name"], "bash");
    assert_eq!(json["function"]["arguments"], "{\"cmd\":\"ls\"}");
}

#[test]
fn streaming_tool_call_skips_none_fields() {
    let tc = StreamingToolCall {
        index: 1,
        id: None,
        call_type: None,
        function: Some(StreamingFunctionCall {
            name: None,
            arguments: Some("{\"partial\":true}".to_string()),
        }),
    };

    let json = serde_json::to_value(&tc).unwrap();
    assert_eq!(json["index"], 1);
    assert!(json.get("id").is_none());
    assert!(json.get("type").is_none());
    assert!(json["function"].get("name").is_none());
    assert_eq!(json["function"]["arguments"], "{\"partial\":true}");
}
