use copilot_adapter::tools::types::*;

// ---------------------------------------------------------------------------
// Tool definition serialization/deserialization
// ---------------------------------------------------------------------------

#[test]
fn tool_deserializes_from_openai_json() {
    let json = serde_json::json!({
        "type": "function",
        "function": {
            "name": "get_weather",
            "description": "Get the current weather",
            "parameters": {
                "type": "object",
                "properties": {
                    "location": {
                        "type": "string",
                        "description": "The city name"
                    }
                },
                "required": ["location"]
            }
        }
    });
    let tool: Tool = serde_json::from_value(json).unwrap();
    assert_eq!(tool.tool_type, "function");
    assert_eq!(tool.function.name, "get_weather");
    assert_eq!(
        tool.function.description,
        Some("Get the current weather".to_string())
    );
    let params = tool.function.parameters.unwrap();
    assert_eq!(params["type"], "object");
    assert_eq!(params["properties"]["location"]["type"], "string");
    assert_eq!(params["required"][0], "location");
}

#[test]
fn tool_deserializes_without_optional_fields() {
    let json = serde_json::json!({
        "type": "function",
        "function": {
            "name": "noop"
        }
    });
    let tool: Tool = serde_json::from_value(json).unwrap();
    assert_eq!(tool.tool_type, "function");
    assert_eq!(tool.function.name, "noop");
    assert!(tool.function.description.is_none());
    assert!(tool.function.parameters.is_none());
}

#[test]
fn tool_serializes_correctly() {
    let tool = Tool {
        tool_type: "function".to_string(),
        function: Function {
            name: "read_file".to_string(),
            description: Some("Read a file".to_string()),
            parameters: Some(serde_json::json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string" }
                },
                "required": ["path"]
            })),
        },
    };
    let json = serde_json::to_value(&tool).unwrap();
    assert_eq!(json["type"], "function");
    assert_eq!(json["function"]["name"], "read_file");
    assert_eq!(json["function"]["description"], "Read a file");
    assert_eq!(json["function"]["parameters"]["type"], "object");
}

#[test]
fn tool_skips_none_optional_fields() {
    let tool = Tool {
        tool_type: "function".to_string(),
        function: Function {
            name: "noop".to_string(),
            description: None,
            parameters: None,
        },
    };
    let json = serde_json::to_value(&tool).unwrap();
    assert!(json["function"].get("description").is_none());
    assert!(json["function"].get("parameters").is_none());
}

#[test]
fn tool_roundtrip() {
    let tool = Tool {
        tool_type: "function".to_string(),
        function: Function {
            name: "bash".to_string(),
            description: Some("Execute a bash command".to_string()),
            parameters: Some(serde_json::json!({
                "type": "object",
                "properties": {
                    "command": { "type": "string" }
                }
            })),
        },
    };
    let json_str = serde_json::to_string(&tool).unwrap();
    let deserialized: Tool = serde_json::from_str(&json_str).unwrap();
    assert_eq!(deserialized.function.name, "bash");
    assert_eq!(
        deserialized.function.description,
        Some("Execute a bash command".to_string())
    );
}

// ---------------------------------------------------------------------------
// ToolCall serialization/deserialization
// ---------------------------------------------------------------------------

#[test]
fn tool_call_serializes_to_openai_format() {
    let tool_call = ToolCall {
        id: Some("call_abc123".to_string()),
        call_type: Some("function".to_string()),
        function: FunctionCall {
            name: Some("get_weather".to_string()),
            arguments: Some(r#"{"location":"London"}"#.to_string()),
        },
    };
    let json = serde_json::to_value(&tool_call).unwrap();
    assert_eq!(json["id"], "call_abc123");
    assert_eq!(json["type"], "function");
    assert_eq!(json["function"]["name"], "get_weather");
    // arguments is a JSON string, not a parsed object
    assert_eq!(json["function"]["arguments"], r#"{"location":"London"}"#);
}

#[test]
fn tool_call_deserializes_from_openai_format() {
    let json = serde_json::json!({
        "id": "call_xyz",
        "type": "function",
        "function": {
            "name": "read_file",
            "arguments": "{\"path\":\"/tmp/test.txt\"}"
        }
    });
    let tool_call: ToolCall = serde_json::from_value(json).unwrap();
    assert_eq!(tool_call.id, Some("call_xyz".to_string()));
    assert_eq!(tool_call.call_type, Some("function".to_string()));
    assert_eq!(tool_call.function.name, Some("read_file".to_string()));
    assert_eq!(
        tool_call.function.arguments,
        Some("{\"path\":\"/tmp/test.txt\"}".to_string())
    );
}

#[test]
fn tool_call_skips_none_optional_fields() {
    let tool_call = ToolCall {
        id: None,
        call_type: None,
        function: FunctionCall {
            name: None,
            arguments: Some("{}".to_string()),
        },
    };
    let json = serde_json::to_value(&tool_call).unwrap();
    assert!(json.get("id").is_none());
    assert!(json.get("type").is_none());
    assert!(json["function"].get("name").is_none());
    assert_eq!(json["function"]["arguments"], "{}");
}

#[test]
fn tool_call_roundtrip() {
    let tool_call = ToolCall {
        id: Some("call_roundtrip".to_string()),
        call_type: Some("function".to_string()),
        function: FunctionCall {
            name: Some("bash".to_string()),
            arguments: Some(r#"{"command":"ls -la"}"#.to_string()),
        },
    };
    let json_str = serde_json::to_string(&tool_call).unwrap();
    let deserialized: ToolCall = serde_json::from_str(&json_str).unwrap();
    assert_eq!(deserialized.id, Some("call_roundtrip".to_string()));
    assert_eq!(deserialized.function.name, Some("bash".to_string()));
    assert_eq!(
        deserialized.function.arguments,
        Some(r#"{"command":"ls -la"}"#.to_string())
    );
}

#[test]
fn tool_call_with_streaming_partial_fields() {
    // In streaming, tool_call delta may only have partial fields
    let json = serde_json::json!({
        "function": {
            "arguments": "{\"pa"
        }
    });
    let tool_call: ToolCall = serde_json::from_value(json).unwrap();
    assert!(tool_call.id.is_none());
    assert!(tool_call.call_type.is_none());
    assert!(tool_call.function.name.is_none());
    assert_eq!(tool_call.function.arguments, Some("{\"pa".to_string()));
}

// ---------------------------------------------------------------------------
// Anthropic tool types
// ---------------------------------------------------------------------------

#[test]
fn anthropic_tool_use_block_serializes() {
    use copilot_adapter::anthropic::types::ContentBlock;

    let block = ContentBlock::ToolUse {
        id: "toolu_abc123".to_string(),
        name: "get_weather".to_string(),
        input: serde_json::json!({"location": "London"}),
        cache_control: None,
    };
    let json = serde_json::to_value(&block).unwrap();
    assert_eq!(json["type"], "tool_use");
    assert_eq!(json["id"], "toolu_abc123");
    assert_eq!(json["name"], "get_weather");
    assert_eq!(json["input"]["location"], "London");
}

#[test]
fn anthropic_tool_use_block_deserializes() {
    use copilot_adapter::anthropic::types::ContentBlock;

    let json = serde_json::json!({
        "type": "tool_use",
        "id": "toolu_xyz",
        "name": "bash",
        "input": {"command": "pwd"}
    });
    let block: ContentBlock = serde_json::from_value(json).unwrap();
    match block {
        ContentBlock::ToolUse {
            id, name, input, ..
        } => {
            assert_eq!(id, "toolu_xyz");
            assert_eq!(name, "bash");
            assert_eq!(input["command"], "pwd");
        }
        _ => panic!("Expected ToolUse variant"),
    }
}

#[test]
fn anthropic_tool_result_block_serializes_with_string() {
    use copilot_adapter::anthropic::types::{ContentBlock, ToolResultContent};

    let block = ContentBlock::ToolResult {
        tool_use_id: "toolu_abc123".to_string(),
        content: ToolResultContent::Text("The weather is sunny.".to_string()),
        cache_control: None,
    };
    let json = serde_json::to_value(&block).unwrap();
    assert_eq!(json["type"], "tool_result");
    assert_eq!(json["tool_use_id"], "toolu_abc123");
    assert_eq!(json["content"], "The weather is sunny.");
}

#[test]
fn anthropic_tool_result_block_deserializes_with_string() {
    use copilot_adapter::anthropic::types::{ContentBlock, ToolResultContent};

    let json = serde_json::json!({
        "type": "tool_result",
        "tool_use_id": "toolu_abc",
        "content": "Result text"
    });
    let block: ContentBlock = serde_json::from_value(json).unwrap();
    match block {
        ContentBlock::ToolResult {
            tool_use_id,
            content,
            ..
        } => {
            assert_eq!(tool_use_id, "toolu_abc");
            match content {
                ToolResultContent::Text(t) => assert_eq!(t, "Result text"),
                _ => panic!("Expected Text variant"),
            }
        }
        _ => panic!("Expected ToolResult variant"),
    }
}

#[test]
fn anthropic_tool_result_block_with_content_blocks() {
    use copilot_adapter::anthropic::types::{ContentBlock, ToolResultContent};

    let json = serde_json::json!({
        "type": "tool_result",
        "tool_use_id": "toolu_xyz",
        "content": [
            {"type": "text", "text": "line 1"},
            {"type": "text", "text": "line 2"}
        ]
    });
    let block: ContentBlock = serde_json::from_value(json).unwrap();
    match block {
        ContentBlock::ToolResult {
            tool_use_id,
            content,
            ..
        } => {
            assert_eq!(tool_use_id, "toolu_xyz");
            match content {
                ToolResultContent::Blocks(blocks) => {
                    assert_eq!(blocks.len(), 2);
                }
                _ => panic!("Expected Blocks variant"),
            }
        }
        _ => panic!("Expected ToolResult variant"),
    }
}

#[test]
fn anthropic_tool_definition_serializes() {
    use copilot_adapter::anthropic::types::{InputSchema, ToolDefinition};

    let tool = ToolDefinition {
        name: "bash".to_string(),
        description: Some("Execute a command".to_string()),
        input_schema: InputSchema {
            schema_type: "object".to_string(),
            properties: Some(serde_json::json!({
                "command": { "type": "string" }
            })),
            required: Some(vec!["command".to_string()]),
        },
    };
    let json = serde_json::to_value(&tool).unwrap();
    assert_eq!(json["name"], "bash");
    assert_eq!(json["description"], "Execute a command");
    assert_eq!(json["input_schema"]["type"], "object");
    assert_eq!(
        json["input_schema"]["properties"]["command"]["type"],
        "string"
    );
    assert_eq!(json["input_schema"]["required"][0], "command");
}

#[test]
fn anthropic_tool_definition_without_optional_fields() {
    use copilot_adapter::anthropic::types::{InputSchema, ToolDefinition};

    let tool = ToolDefinition {
        name: "noop".to_string(),
        description: None,
        input_schema: InputSchema {
            schema_type: "object".to_string(),
            properties: None,
            required: None,
        },
    };
    let json = serde_json::to_value(&tool).unwrap();
    assert_eq!(json["name"], "noop");
    assert!(json.get("description").is_none());
    assert_eq!(json["input_schema"]["type"], "object");
    assert!(json["input_schema"].get("properties").is_none());
    assert!(json["input_schema"].get("required").is_none());
}

#[test]
fn anthropic_tool_definition_roundtrip() {
    use copilot_adapter::anthropic::types::{InputSchema, ToolDefinition};

    let tool = ToolDefinition {
        name: "read_file".to_string(),
        description: Some("Read file contents".to_string()),
        input_schema: InputSchema {
            schema_type: "object".to_string(),
            properties: Some(serde_json::json!({
                "path": { "type": "string" }
            })),
            required: Some(vec!["path".to_string()]),
        },
    };
    let json_str = serde_json::to_string(&tool).unwrap();
    let deserialized: ToolDefinition = serde_json::from_str(&json_str).unwrap();
    assert_eq!(deserialized.name, "read_file");
    assert_eq!(
        deserialized.description,
        Some("Read file contents".to_string())
    );
    assert_eq!(deserialized.input_schema.schema_type, "object");
    assert_eq!(
        deserialized.input_schema.required,
        Some(vec!["path".to_string()])
    );
}

// ---------------------------------------------------------------------------
// Integration: tools field on request types
// ---------------------------------------------------------------------------

#[test]
fn chat_completion_request_with_tools_deserializes() {
    use copilot_adapter::copilot::types::ChatCompletionRequest;

    let json = serde_json::json!({
        "model": "gpt-4",
        "messages": [{"role": "user", "content": "Hello"}],
        "tools": [{
            "type": "function",
            "function": {
                "name": "get_weather",
                "description": "Get weather",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "location": {"type": "string"}
                    }
                }
            }
        }],
        "tool_choice": "auto"
    });
    let req: ChatCompletionRequest = serde_json::from_value(json).unwrap();
    assert!(req.tools.is_some());
    let tools = req.tools.unwrap();
    assert_eq!(tools.len(), 1);
    assert_eq!(tools[0].function.name, "get_weather");
    assert_eq!(req.tool_choice, Some(serde_json::json!("auto")));
}

#[test]
fn chat_completion_request_without_tools_skips() {
    use copilot_adapter::copilot::types::ChatCompletionRequest;

    let req = ChatCompletionRequest {
        model: "gpt-4".to_string(),
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
    assert!(json.get("tools").is_none());
    assert!(json.get("tool_choice").is_none());
}

#[test]
fn message_with_tool_calls_serializes() {
    use copilot_adapter::copilot::types::{Message, MessageContent};

    let msg = Message {
        role: "assistant".to_string(),
        content: MessageContent::Text("".to_string()),
        name: None,
        tool_calls: Some(vec![ToolCall {
            id: Some("call_123".to_string()),
            call_type: Some("function".to_string()),
            function: FunctionCall {
                name: Some("bash".to_string()),
                arguments: Some(r#"{"command":"ls"}"#.to_string()),
            },
        }]),
        tool_call_id: None,
    };
    let json = serde_json::to_value(&msg).unwrap();
    assert_eq!(json["tool_calls"][0]["id"], "call_123");
    assert_eq!(json["tool_calls"][0]["function"]["name"], "bash");
}

#[test]
fn message_with_tool_role_deserializes() {
    use copilot_adapter::copilot::types::Message;

    let json = serde_json::json!({
        "role": "tool",
        "content": "Result of tool call",
        "tool_call_id": "call_abc"
    });
    let msg: Message = serde_json::from_value(json).unwrap();
    assert_eq!(msg.role, "tool");
    assert_eq!(msg.tool_call_id, Some("call_abc".to_string()));
}

#[test]
fn anthropic_request_with_tools_deserializes() {
    use copilot_adapter::anthropic::types::AnthropicRequest;

    let json = serde_json::json!({
        "model": "claude-sonnet-4-20250514",
        "max_tokens": 1024,
        "messages": [{"role": "user", "content": "Hello"}],
        "tools": [{
            "name": "bash",
            "description": "Run a command",
            "input_schema": {
                "type": "object",
                "properties": {
                    "command": {"type": "string"}
                },
                "required": ["command"]
            }
        }]
    });
    let req: AnthropicRequest = serde_json::from_value(json).unwrap();
    assert!(req.tools.is_some());
    let tools = req.tools.unwrap();
    assert_eq!(tools.len(), 1);
    assert_eq!(tools[0].name, "bash");
    assert_eq!(tools[0].input_schema.schema_type, "object");
}

#[test]
fn anthropic_request_without_tools_skips() {
    use copilot_adapter::anthropic::types::AnthropicRequest;

    let json = serde_json::json!({
        "model": "claude-sonnet-4-20250514",
        "max_tokens": 1024,
        "messages": [{"role": "user", "content": "Hello"}]
    });
    let req: AnthropicRequest = serde_json::from_value(json).unwrap();
    assert!(req.tools.is_none());

    // Serializing back should omit tools
    let out = serde_json::to_value(&req).unwrap();
    assert!(out.get("tools").is_none());
}
