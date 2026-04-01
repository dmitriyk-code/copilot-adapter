//! Unit tests for native OpenAI tool call type handling.
//!
//! Epic 0 verification: confirms that the existing type system correctly
//! handles native OpenAI-format tool call responses (both streaming and
//! non-streaming). These tests validate serialization, deserialization,
//! and edge cases for the `tool_calls` format that the Copilot API returns
//! when native tools are passed in the request.
//!
//! Epic 2 update: `MessageContent` now handles `null` content by treating it
//! as empty text. `ChunkDelta.tool_calls` now uses `StreamingToolCall` with
//! an `index` field. `ChatCompletionRequest.tools` now uses `OpenAITool`.

use copilot_adapter::copilot::types::*;
use copilot_adapter::tools::types::*;

// ===========================================================================
// E0-T2: Document/verify response format for tool_calls
// ===========================================================================

#[test]
fn native_tool_call_response_with_empty_string_content_deserializes() {
    // Native tool call response with empty string content (workaround for
    // the null content issue — some APIs return "" instead of null).
    let json = serde_json::json!({
        "id": "chatcmpl-abc123",
        "object": "chat.completion",
        "created": 1700000000,
        "model": "gpt-4o",
        "choices": [{
            "index": 0,
            "message": {
                "role": "assistant",
                "content": "",
                "tool_calls": [{
                    "id": "call_abc123",
                    "type": "function",
                    "function": {
                        "name": "get_weather",
                        "arguments": "{\"location\":\"London\"}"
                    }
                }]
            },
            "finish_reason": "tool_calls"
        }],
        "usage": {
            "prompt_tokens": 20,
            "completion_tokens": 10,
            "total_tokens": 30
        }
    });

    let resp: ChatCompletionResponse = serde_json::from_value(json).unwrap();
    assert_eq!(resp.id, "chatcmpl-abc123");
    assert_eq!(resp.model, "gpt-4o");
    assert_eq!(resp.choices.len(), 1);

    let choice = &resp.choices[0];
    assert_eq!(choice.finish_reason, Some("tool_calls".to_string()));

    let tool_calls = choice.message.tool_calls.as_ref().unwrap();
    assert_eq!(tool_calls.len(), 1);

    let tc = &tool_calls[0];
    assert_eq!(tc.id, Some("call_abc123".to_string()));
    assert_eq!(tc.call_type, Some("function".to_string()));
    assert_eq!(tc.function.name, Some("get_weather".to_string()));
    assert_eq!(
        tc.function.arguments,
        Some("{\"location\":\"London\"}".to_string())
    );
}

#[test]
fn null_content_deserialization_succeeds() {
    // Fixed in Epic 2: MessageContent now handles null content by treating
    // it as empty text. Native tool call responses from OpenAI use
    // `"content": null`, which previously failed to deserialize.
    let json = serde_json::json!({
        "id": "chatcmpl-null",
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
                        "arguments": "{\"command\":\"ls\"}"
                    }
                }]
            },
            "finish_reason": "tool_calls"
        }]
    });

    let resp: ChatCompletionResponse = serde_json::from_value(json).unwrap();
    assert_eq!(resp.id, "chatcmpl-null");

    let choice = &resp.choices[0];
    // null content deserializes as empty text
    assert_eq!(choice.message.content.as_text(), "");

    let tool_calls = choice.message.tool_calls.as_ref().unwrap();
    assert_eq!(tool_calls.len(), 1);
    assert_eq!(tool_calls[0].function.name, Some("bash".to_string()));
}

#[test]
fn native_multi_tool_call_response_with_empty_content_deserializes() {
    // Multiple tool calls with empty string content (not null).
    let json = serde_json::json!({
        "id": "chatcmpl-multi",
        "object": "chat.completion",
        "created": 1700000000,
        "model": "gpt-4o",
        "choices": [{
            "index": 0,
            "message": {
                "role": "assistant",
                "content": "",
                "tool_calls": [
                    {
                        "id": "call_001",
                        "type": "function",
                        "function": {
                            "name": "get_weather",
                            "arguments": "{\"location\":\"London\"}"
                        }
                    },
                    {
                        "id": "call_002",
                        "type": "function",
                        "function": {
                            "name": "read_file",
                            "arguments": "{\"path\":\"/tmp/test.txt\"}"
                        }
                    }
                ]
            },
            "finish_reason": "tool_calls"
        }]
    });

    let resp: ChatCompletionResponse = serde_json::from_value(json).unwrap();
    let tool_calls = resp.choices[0].message.tool_calls.as_ref().unwrap();
    assert_eq!(tool_calls.len(), 2);

    assert_eq!(tool_calls[0].function.name, Some("get_weather".to_string()));
    assert_eq!(tool_calls[1].function.name, Some("read_file".to_string()));

    // Each call should have a unique ID
    assert_ne!(tool_calls[0].id, tool_calls[1].id);
}

#[test]
fn native_tool_call_preserves_typed_arguments() {
    // This is the key test for Epic 0: native tool calls preserve types.
    // When using native tools, arguments are a JSON string that the caller
    // must parse — but the JSON itself preserves number/boolean types.
    let json = serde_json::json!({
        "id": "chatcmpl-typed",
        "object": "chat.completion",
        "created": 1700000000,
        "model": "gpt-4o",
        "choices": [{
            "index": 0,
            "message": {
                "role": "assistant",
                "content": "",
                "tool_calls": [{
                    "id": "call_typed",
                    "type": "function",
                    "function": {
                        "name": "search",
                        "arguments": "{\"query\":\"test\",\"limit\":10,\"recursive\":true}"
                    }
                }]
            },
            "finish_reason": "tool_calls"
        }]
    });

    let resp: ChatCompletionResponse = serde_json::from_value(json).unwrap();
    let tc = &resp.choices[0].message.tool_calls.as_ref().unwrap()[0];

    // Parse the arguments JSON string to verify type preservation
    let args: serde_json::Value =
        serde_json::from_str(tc.function.arguments.as_ref().unwrap()).unwrap();

    assert_eq!(args["query"], "test");
    assert_eq!(args["limit"], 10); // number, not string "10"
    assert_eq!(args["recursive"], true); // boolean, not string "true"
}

// ===========================================================================
// E0-T3: Streaming chunk types for native tool calls
// ===========================================================================

#[test]
fn streaming_chunk_with_tool_calls_delta_deserializes() {
    // First streaming chunk for a tool call: contains id, type, and function name.
    let json = serde_json::json!({
        "id": "chatcmpl-stream123",
        "object": "chat.completion.chunk",
        "created": 1700000000,
        "model": "gpt-4o",
        "choices": [{
            "index": 0,
            "delta": {
                "tool_calls": [{
                    "index": 0,
                    "id": "call_stream_001",
                    "type": "function",
                    "function": {
                        "name": "get_weather",
                        "arguments": ""
                    }
                }]
            },
            "finish_reason": null
        }]
    });

    let chunk: ChatCompletionChunk = serde_json::from_value(json).unwrap();
    assert_eq!(chunk.id, "chatcmpl-stream123");

    let delta = &chunk.choices[0].delta;
    let tool_calls = delta.tool_calls.as_ref().unwrap();
    assert_eq!(tool_calls.len(), 1);

    let tc = &tool_calls[0];
    assert_eq!(tc.index, 0);
    assert_eq!(tc.id, Some("call_stream_001".to_string()));
    assert_eq!(tc.call_type, Some("function".to_string()));
    let func = tc.function.as_ref().unwrap();
    assert_eq!(func.name, Some("get_weather".to_string()));
    assert_eq!(func.arguments, Some("".to_string()));
}

#[test]
fn streaming_chunk_with_partial_arguments_deserializes() {
    // Subsequent chunks only have partial arguments (no id/type/name).
    let json = serde_json::json!({
        "id": "chatcmpl-stream123",
        "object": "chat.completion.chunk",
        "created": 1700000000,
        "model": "gpt-4o",
        "choices": [{
            "index": 0,
            "delta": {
                "tool_calls": [{
                    "index": 0,
                    "function": {
                        "arguments": "{\"location\":"
                    }
                }]
            },
            "finish_reason": null
        }]
    });

    let chunk: ChatCompletionChunk = serde_json::from_value(json).unwrap();
    let tc = &chunk.choices[0].delta.tool_calls.as_ref().unwrap()[0];

    // Only partial data — id and name are absent
    assert!(tc.id.is_none());
    assert!(tc.call_type.is_none());
    let func = tc.function.as_ref().unwrap();
    assert!(func.name.is_none());
    assert_eq!(func.arguments, Some("{\"location\":".to_string()));
}

#[test]
fn streaming_tool_call_finish_reason_is_tool_calls() {
    // The final chunk has finish_reason "tool_calls" (not "stop").
    let json = serde_json::json!({
        "id": "chatcmpl-stream123",
        "object": "chat.completion.chunk",
        "created": 1700000000,
        "model": "gpt-4o",
        "choices": [{
            "index": 0,
            "delta": {},
            "finish_reason": "tool_calls"
        }]
    });

    let chunk: ChatCompletionChunk = serde_json::from_value(json).unwrap();
    assert_eq!(
        chunk.choices[0].finish_reason,
        Some("tool_calls".to_string())
    );
}

#[test]
fn streaming_chunks_can_reconstruct_full_tool_call() {
    // Simulate a complete streaming sequence and reconstruct the tool call.
    let chunks_json = vec![
        // 1. Role
        serde_json::json!({
            "id": "chatcmpl-recon",
            "object": "chat.completion.chunk",
            "created": 1700000000,
            "model": "gpt-4o",
            "choices": [{"index": 0, "delta": {"role": "assistant"}, "finish_reason": null}]
        }),
        // 2. Tool call start
        serde_json::json!({
            "id": "chatcmpl-recon",
            "object": "chat.completion.chunk",
            "created": 1700000000,
            "model": "gpt-4o",
            "choices": [{"index": 0, "delta": {
                "tool_calls": [{
                    "index": 0,
                    "id": "call_recon_001",
                    "type": "function",
                    "function": {"name": "get_weather", "arguments": ""}
                }]
            }, "finish_reason": null}]
        }),
        // 3. Arguments part 1
        serde_json::json!({
            "id": "chatcmpl-recon",
            "object": "chat.completion.chunk",
            "created": 1700000000,
            "model": "gpt-4o",
            "choices": [{"index": 0, "delta": {
                "tool_calls": [{"index": 0, "function": {"arguments": "{\"loc"}}]
            }, "finish_reason": null}]
        }),
        // 4. Arguments part 2
        serde_json::json!({
            "id": "chatcmpl-recon",
            "object": "chat.completion.chunk",
            "created": 1700000000,
            "model": "gpt-4o",
            "choices": [{"index": 0, "delta": {
                "tool_calls": [{"index": 0, "function": {"arguments": "ation\":\"London\"}"}}]
            }, "finish_reason": null}]
        }),
        // 5. Finish
        serde_json::json!({
            "id": "chatcmpl-recon",
            "object": "chat.completion.chunk",
            "created": 1700000000,
            "model": "gpt-4o",
            "choices": [{"index": 0, "delta": {}, "finish_reason": "tool_calls"}]
        }),
    ];

    // Parse all chunks
    let chunks: Vec<ChatCompletionChunk> = chunks_json
        .into_iter()
        .map(|j| serde_json::from_value(j).unwrap())
        .collect();

    assert_eq!(chunks.len(), 5);

    // Reconstruct the tool call by accumulating deltas
    let mut call_id = String::new();
    let mut call_name = String::new();
    let mut call_args = String::new();

    for chunk in &chunks {
        if let Some(tool_calls) = &chunk.choices[0].delta.tool_calls {
            for tc in tool_calls {
                if let Some(id) = &tc.id {
                    call_id = id.clone();
                }
                if let Some(ref func) = tc.function {
                    if let Some(name) = &func.name {
                        call_name = name.clone();
                    }
                    if let Some(args) = &func.arguments {
                        call_args.push_str(args);
                    }
                }
            }
        }
    }

    assert_eq!(call_id, "call_recon_001");
    assert_eq!(call_name, "get_weather");
    assert_eq!(call_args, "{\"location\":\"London\"}");

    // Verify the reconstructed arguments parse as valid JSON with correct types
    let parsed_args: serde_json::Value = serde_json::from_str(&call_args).unwrap();
    assert_eq!(parsed_args["location"], "London");
}

// ===========================================================================
// E0-T4: Tool name length and request serialization
// ===========================================================================

#[test]
fn request_with_native_tools_serializes_correctly() {
    // Verify that ChatCompletionRequest with tools serializes the tools
    // field (they'll be needed when forwarded to the Copilot API).
    let req = ChatCompletionRequest {
        model: "gpt-4o".to_string(),
        messages: vec![Message {
            role: "user".to_string(),
            content: MessageContent::Text("Get the weather".to_string()),
            name: None,
            tool_calls: None,
            tool_call_id: None,
        }],
        stream: Some(false),
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
                name: "get_weather".to_string(),
                description: Some("Get weather for a location".to_string()),
                parameters: Some(serde_json::json!({
                    "type": "object",
                    "properties": {
                        "location": {"type": "string"}
                    },
                    "required": ["location"]
                })),
            },
        }]),
        tool_choice: Some(serde_json::json!("auto")),
    };

    let json = serde_json::to_value(&req).unwrap();

    // tools should be present in serialized output
    let tools = json["tools"].as_array().unwrap();
    assert_eq!(tools.len(), 1);
    assert_eq!(tools[0]["type"], "function");
    assert_eq!(tools[0]["function"]["name"], "get_weather");
    assert_eq!(tools[0]["function"]["parameters"]["type"], "object");

    // tool_choice should be present
    assert_eq!(json["tool_choice"], "auto");
}

#[test]
fn tool_name_at_64_chars_is_valid() {
    // 64 characters is OpenAI's documented limit.
    let name = "a".repeat(64);
    let tool = Tool {
        tool_type: "function".to_string(),
        function: Function {
            name: name.clone(),
            description: Some("Test".to_string()),
            parameters: None,
        },
    };

    let json = serde_json::to_value(&tool).unwrap();
    assert_eq!(json["function"]["name"].as_str().unwrap().len(), 64);
}

#[test]
fn tool_name_over_64_chars_serializes_but_may_need_truncation() {
    // A tool name over 64 chars can be serialized but may be rejected by
    // the API. This test documents the behavior we need to handle.
    let name = "mcp__very_long_server_name__extremely_detailed_tool_description_that_exceeds_the_openai_limit_test";
    assert!(name.len() > 64);

    let tool = Tool {
        tool_type: "function".to_string(),
        function: Function {
            name: name.to_string(),
            description: Some("Test".to_string()),
            parameters: None,
        },
    };

    let json = serde_json::to_value(&tool).unwrap();
    // It serializes fine — but the API may reject it
    assert_eq!(json["function"]["name"].as_str().unwrap(), name);
}

// ===========================================================================
// Edge cases
// ===========================================================================

#[test]
fn response_with_text_and_tool_calls_deserializes() {
    // Some models may return both content text AND tool_calls.
    let json = serde_json::json!({
        "id": "chatcmpl-mixed",
        "object": "chat.completion",
        "created": 1700000000,
        "model": "gpt-4o",
        "choices": [{
            "index": 0,
            "message": {
                "role": "assistant",
                "content": "I'll check the weather for you.",
                "tool_calls": [{
                    "id": "call_mixed_001",
                    "type": "function",
                    "function": {
                        "name": "get_weather",
                        "arguments": "{\"location\":\"London\"}"
                    }
                }]
            },
            "finish_reason": "tool_calls"
        }]
    });

    let resp: ChatCompletionResponse = serde_json::from_value(json).unwrap();
    let msg = &resp.choices[0].message;

    // Both text content and tool_calls are present
    assert_eq!(msg.content.as_text(), "I'll check the weather for you.");
    assert!(msg.tool_calls.is_some());
    assert_eq!(msg.tool_calls.as_ref().unwrap().len(), 1);
}

#[test]
fn streaming_chunk_with_content_and_no_tool_calls() {
    // Regular text chunk (no tool_calls) should still deserialize fine.
    let json = serde_json::json!({
        "id": "chatcmpl-text",
        "object": "chat.completion.chunk",
        "created": 1700000000,
        "model": "gpt-4o",
        "choices": [{
            "index": 0,
            "delta": {"content": "Hello!"},
            "finish_reason": null
        }]
    });

    let chunk: ChatCompletionChunk = serde_json::from_value(json).unwrap();
    assert_eq!(chunk.choices[0].delta.content, Some("Hello!".to_string()));
    assert!(chunk.choices[0].delta.tool_calls.is_none());
}

#[test]
fn tool_call_arguments_with_nested_json_deserializes() {
    // Tool arguments can contain nested JSON structures.
    let nested_args = r#"{"query":"test","options":{"limit":10,"offset":0,"filters":{"type":"file","extension":".rs"}}}"#;
    let json = serde_json::json!({
        "id": "chatcmpl-nested",
        "object": "chat.completion",
        "created": 1700000000,
        "model": "gpt-4o",
        "choices": [{
            "index": 0,
            "message": {
                "role": "assistant",
                "content": "",
                "tool_calls": [{
                    "id": "call_nested",
                    "type": "function",
                    "function": {
                        "name": "search",
                        "arguments": nested_args
                    }
                }]
            },
            "finish_reason": "tool_calls"
        }]
    });

    let resp: ChatCompletionResponse = serde_json::from_value(json).unwrap();
    let tc = &resp.choices[0].message.tool_calls.as_ref().unwrap()[0];

    let parsed: serde_json::Value =
        serde_json::from_str(tc.function.arguments.as_ref().unwrap()).unwrap();
    assert_eq!(parsed["options"]["limit"], 10);
    assert_eq!(parsed["options"]["filters"]["extension"], ".rs");
}

#[test]
fn tool_call_with_empty_arguments() {
    // A tool with no required parameters may have empty arguments.
    let json = serde_json::json!({
        "id": "chatcmpl-empty",
        "object": "chat.completion",
        "created": 1700000000,
        "model": "gpt-4o",
        "choices": [{
            "index": 0,
            "message": {
                "role": "assistant",
                "content": "",
                "tool_calls": [{
                    "id": "call_empty",
                    "type": "function",
                    "function": {
                        "name": "list_files",
                        "arguments": "{}"
                    }
                }]
            },
            "finish_reason": "tool_calls"
        }]
    });

    let resp: ChatCompletionResponse = serde_json::from_value(json).unwrap();
    let tc = &resp.choices[0].message.tool_calls.as_ref().unwrap()[0];
    assert_eq!(tc.function.arguments, Some("{}".to_string()));
}
