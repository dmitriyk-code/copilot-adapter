use copilot_adapter::copilot::types::{Message, MessageContent};
use copilot_adapter::tools::injector::{
    format_tools_as_json, inject_tools_into_messages, translate_tool_messages,
    TOOL_USAGE_INSTRUCTIONS,
};
use copilot_adapter::tools::types::{Function, Tool};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn sample_tool(name: &str, desc: Option<&str>, params: Option<serde_json::Value>) -> Tool {
    Tool {
        tool_type: "function".into(),
        function: Function {
            name: name.into(),
            description: desc.map(|s| s.into()),
            parameters: params,
        },
    }
}

fn make_message(role: &str, content: &str) -> Message {
    Message {
        role: role.into(),
        content: MessageContent::Text(content.into()),
        name: None,
        tool_calls: None,
        tool_call_id: None,
    }
}

fn make_tool_message(content: &str, call_id: &str) -> Message {
    Message {
        role: "tool".into(),
        content: MessageContent::Text(content.into()),
        name: None,
        tool_calls: None,
        tool_call_id: Some(call_id.into()),
    }
}

// ---------------------------------------------------------------------------
// E2-T7: Tools formatted as valid JSON
// ---------------------------------------------------------------------------

#[test]
fn format_tools_produces_valid_json_single_tool() {
    let tools = vec![sample_tool(
        "get_weather",
        Some("Get the current weather"),
        Some(serde_json::json!({
            "type": "object",
            "properties": {
                "location": { "type": "string", "description": "City name" }
            },
            "required": ["location"]
        })),
    )];

    let output = format_tools_as_json(&tools);
    let parsed: serde_json::Value = serde_json::from_str(&output).unwrap();

    assert!(parsed["functions"].is_array());
    assert_eq!(parsed["functions"].as_array().unwrap().len(), 1);
    assert_eq!(parsed["functions"][0]["name"], "get_weather");
    assert_eq!(parsed["functions"][0]["description"], "Get the current weather");
    assert_eq!(parsed["functions"][0]["parameters"]["type"], "object");
    assert_eq!(
        parsed["functions"][0]["parameters"]["properties"]["location"]["type"],
        "string"
    );
}

#[test]
fn format_tools_produces_valid_json_multiple_tools() {
    let tools = vec![
        sample_tool(
            "read_file",
            Some("Read a file"),
            Some(serde_json::json!({
                "type": "object",
                "properties": { "path": { "type": "string" } },
                "required": ["path"]
            })),
        ),
        sample_tool(
            "write_file",
            Some("Write a file"),
            Some(serde_json::json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string" },
                    "content": { "type": "string" }
                },
                "required": ["path", "content"]
            })),
        ),
    ];

    let output = format_tools_as_json(&tools);
    let parsed: serde_json::Value = serde_json::from_str(&output).unwrap();

    let funcs = parsed["functions"].as_array().unwrap();
    assert_eq!(funcs.len(), 2);
    assert_eq!(funcs[0]["name"], "read_file");
    assert_eq!(funcs[1]["name"], "write_file");
}

#[test]
fn format_tools_omits_missing_optional_fields() {
    let tools = vec![sample_tool("noop", None, None)];

    let output = format_tools_as_json(&tools);
    let parsed: serde_json::Value = serde_json::from_str(&output).unwrap();

    let func = &parsed["functions"][0];
    assert_eq!(func["name"], "noop");
    assert!(func.get("description").is_none());
    assert!(func.get("parameters").is_none());
}

#[test]
fn format_tools_empty_list_produces_empty_array() {
    let tools: Vec<Tool> = vec![];
    let output = format_tools_as_json(&tools);
    let parsed: serde_json::Value = serde_json::from_str(&output).unwrap();

    assert!(parsed["functions"].as_array().unwrap().is_empty());
}

// ---------------------------------------------------------------------------
// E2-T8: Injection prepends to existing system message
// ---------------------------------------------------------------------------

#[test]
fn inject_prepends_to_existing_system_message() {
    let tools = vec![sample_tool(
        "bash",
        Some("Execute a command"),
        Some(serde_json::json!({
            "type": "object",
            "properties": { "command": { "type": "string" } },
            "required": ["command"]
        })),
    )];

    let mut messages = vec![
        make_message("system", "You are a helpful assistant."),
        make_message("user", "Hello"),
    ];

    inject_tools_into_messages(&mut messages, &tools);

    // Still exactly 2 messages
    assert_eq!(messages.len(), 2);

    // System message is first
    assert_eq!(messages[0].role, "system");

    let system_content = messages[0].content.as_text();

    // Original content preserved at the end
    assert!(system_content.ends_with("You are a helpful assistant."));

    // Tool definitions appear at the start
    assert!(system_content.starts_with("# Available Functions"));

    // Contains the formatted tool JSON
    assert!(system_content.contains("bash"));
    assert!(system_content.contains("Execute a command"));

    // Contains usage instructions
    assert!(system_content.contains("# How to Call Functions"));
    assert!(system_content.contains("function_call"));
}

#[test]
fn inject_preserves_user_message_content() {
    let tools = vec![sample_tool("noop", None, None)];
    let mut messages = vec![
        make_message("system", "Be concise."),
        make_message("user", "What time is it?"),
    ];

    inject_tools_into_messages(&mut messages, &tools);

    assert_eq!(messages[1].role, "user");
    assert_eq!(messages[1].content.as_text(), "What time is it?");
}

#[test]
fn inject_with_empty_tools_does_nothing() {
    let tools: Vec<Tool> = vec![];
    let mut messages = vec![
        make_message("system", "You are helpful."),
        make_message("user", "Hi"),
    ];

    inject_tools_into_messages(&mut messages, &tools);

    assert_eq!(messages.len(), 2);
    assert_eq!(messages[0].content.as_text(), "You are helpful.");
}

// ---------------------------------------------------------------------------
// E2-T9: Injection creates system message if missing
// ---------------------------------------------------------------------------

#[test]
fn inject_creates_system_message_when_missing() {
    let tools = vec![sample_tool(
        "get_weather",
        Some("Get weather"),
        Some(serde_json::json!({
            "type": "object",
            "properties": { "location": { "type": "string" } }
        })),
    )];

    let mut messages = vec![make_message("user", "What's the weather in London?")];

    inject_tools_into_messages(&mut messages, &tools);

    // Now 2 messages — system was inserted at index 0
    assert_eq!(messages.len(), 2);
    assert_eq!(messages[0].role, "system");
    assert_eq!(messages[1].role, "user");

    let system_content = messages[0].content.as_text();
    assert!(system_content.contains("# Available Functions"));
    assert!(system_content.contains("get_weather"));
    assert!(system_content.contains("# How to Call Functions"));
}

#[test]
fn inject_creates_system_message_preserving_message_order() {
    let tools = vec![sample_tool("noop", None, None)];

    let mut messages = vec![
        make_message("user", "Hello"),
        make_message("assistant", "Hi there!"),
        make_message("user", "Do something"),
    ];

    inject_tools_into_messages(&mut messages, &tools);

    assert_eq!(messages.len(), 4);
    assert_eq!(messages[0].role, "system");
    assert_eq!(messages[1].role, "user");
    assert_eq!(messages[1].content.as_text(), "Hello");
    assert_eq!(messages[2].role, "assistant");
    assert_eq!(messages[3].role, "user");
}

// ---------------------------------------------------------------------------
// E2-T10: Tool role messages translated to user messages
// ---------------------------------------------------------------------------

#[test]
fn translate_tool_messages_converts_to_user_role() {
    let mut messages = vec![
        make_message("user", "What is the weather?"),
        make_message("assistant", "Let me check..."),
        make_tool_message("The weather is sunny, 72°F.", "call_abc123"),
    ];

    translate_tool_messages(&mut messages);

    // Tool message converted to user
    assert_eq!(messages[2].role, "user");
    // tool_call_id cleared
    assert!(messages[2].tool_call_id.is_none());

    let content = messages[2].content.as_text();
    assert!(content.contains("call_abc123"));
    assert!(content.contains("The weather is sunny, 72°F."));
}

#[test]
fn translate_tool_messages_preserves_non_tool_messages() {
    let mut messages = vec![
        make_message("system", "You are helpful."),
        make_message("user", "Hi"),
        make_message("assistant", "Hello!"),
    ];

    translate_tool_messages(&mut messages);

    assert_eq!(messages[0].role, "system");
    assert_eq!(messages[0].content.as_text(), "You are helpful.");
    assert_eq!(messages[1].role, "user");
    assert_eq!(messages[1].content.as_text(), "Hi");
    assert_eq!(messages[2].role, "assistant");
    assert_eq!(messages[2].content.as_text(), "Hello!");
}

#[test]
fn translate_multiple_tool_messages() {
    let mut messages = vec![
        make_message("user", "Do two things"),
        make_message("assistant", "Sure"),
        make_tool_message("Result 1", "call_001"),
        make_tool_message("Result 2", "call_002"),
    ];

    translate_tool_messages(&mut messages);

    assert_eq!(messages[2].role, "user");
    assert!(messages[2].content.as_text().contains("call_001"));
    assert!(messages[2].content.as_text().contains("Result 1"));

    assert_eq!(messages[3].role, "user");
    assert!(messages[3].content.as_text().contains("call_002"));
    assert!(messages[3].content.as_text().contains("Result 2"));
}

#[test]
fn translate_tool_message_without_call_id() {
    let mut messages = vec![Message {
        role: "tool".into(),
        content: MessageContent::Text("Some result".into()),
        name: None,
        tool_calls: None,
        tool_call_id: None,
    }];

    translate_tool_messages(&mut messages);

    assert_eq!(messages[0].role, "user");
    let content = messages[0].content.as_text();
    // Should still work, just with empty call_id
    assert!(content.contains("Tool Result"));
    assert!(content.contains("Some result"));
}

// ---------------------------------------------------------------------------
// Additional edge cases
// ---------------------------------------------------------------------------

#[test]
fn tool_usage_instructions_constant_mentions_json_fencing() {
    assert!(TOOL_USAGE_INSTRUCTIONS.contains("```json"));
    assert!(TOOL_USAGE_INSTRUCTIONS.contains("function_call"));
    assert!(TOOL_USAGE_INSTRUCTIONS.contains("arguments"));
}

#[test]
fn inject_with_multiple_tools_includes_all() {
    let tools = vec![
        sample_tool("tool_a", Some("First tool"), None),
        sample_tool("tool_b", Some("Second tool"), None),
        sample_tool("tool_c", Some("Third tool"), None),
    ];

    let mut messages = vec![make_message("system", "Base instructions.")];

    inject_tools_into_messages(&mut messages, &tools);

    let content = messages[0].content.as_text();
    assert!(content.contains("tool_a"));
    assert!(content.contains("tool_b"));
    assert!(content.contains("tool_c"));
    assert!(content.contains("First tool"));
    assert!(content.contains("Second tool"));
    assert!(content.contains("Third tool"));
    // Original preserved
    assert!(content.contains("Base instructions."));
}
