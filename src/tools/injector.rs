use crate::copilot::types::{Message, MessageContent};
use crate::tools::types::Tool;

// ---------------------------------------------------------------------------
// Tool usage instructions
// ---------------------------------------------------------------------------

/// Instructions appended after tool definitions telling the model how to
/// format tool calls. The parser (Epic 3) will look for JSON blocks matching
/// this exact format.
pub const TOOL_USAGE_INSTRUCTIONS: &str = r#"
# How to Call Functions

When you need to call a function, respond with a JSON block inside a fenced code block:

```json
{"function_call": {"name": "function_name", "arguments": {"param": "value"}}}
```

Rules:
- Always wrap the JSON in a ```json fenced code block.
- "name" must exactly match one of the function names listed above.
- "arguments" must be a JSON object matching the function's parameter schema.
- You may call multiple functions by including multiple fenced JSON blocks.
- After the function results are provided, continue your response normally."#;

// ---------------------------------------------------------------------------
// Format tools as JSON
// ---------------------------------------------------------------------------

/// Convert a slice of `Tool` definitions into a human-readable JSON prompt
/// that can be injected into the system message.
///
/// The output lists each function with its name, description, and parameter
/// schema so the model knows what tools are available.
pub fn format_tools_as_json(tools: &[Tool]) -> String {
    let funcs: Vec<serde_json::Value> = tools
        .iter()
        .map(|t| {
            let mut obj = serde_json::Map::new();
            obj.insert("name".into(), serde_json::Value::String(t.function.name.clone()));
            if let Some(ref desc) = t.function.description {
                obj.insert("description".into(), serde_json::Value::String(desc.clone()));
            }
            if let Some(ref params) = t.function.parameters {
                obj.insert("parameters".into(), params.clone());
            }
            serde_json::Value::Object(obj)
        })
        .collect();

    let wrapper = serde_json::json!({ "functions": funcs });
    serde_json::to_string_pretty(&wrapper).expect("tool JSON serialization should not fail")
}

// ---------------------------------------------------------------------------
// Inject tools into messages
// ---------------------------------------------------------------------------

/// Inject tool definitions and usage instructions into the message list.
///
/// The function prepends the tool definitions (as formatted JSON) and the
/// usage instructions to the **first system message** found in `messages`.
/// If no system message exists, a new one is created and inserted at index 0.
///
/// The original system message content is preserved — the tool block is
/// prepended so it appears before any existing instructions.
pub fn inject_tools_into_messages(messages: &mut Vec<Message>, tools: &[Tool]) {
    if tools.is_empty() {
        return;
    }

    let tool_prompt = build_tool_prompt(tools);

    if let Some(system_msg) = messages.iter_mut().find(|m| m.role == "system") {
        // Prepend tool definitions to the existing system message content.
        let existing = system_msg.content.as_text();
        system_msg.content = MessageContent::Text(format!("{tool_prompt}\n\n{existing}"));
    } else {
        // No system message — create one containing only the tool prompt.
        messages.insert(
            0,
            Message {
                role: "system".into(),
                content: MessageContent::Text(tool_prompt),
                name: None,
                tool_calls: None,
                tool_call_id: None,
            },
        );
    }
}

/// Build the full prompt block that gets injected into the system message.
fn build_tool_prompt(tools: &[Tool]) -> String {
    let json = format_tools_as_json(tools);
    format!(
        "# Available Functions\n\nYou have access to the following functions:\n\n{json}\n{TOOL_USAGE_INSTRUCTIONS}"
    )
}

// ---------------------------------------------------------------------------
// Translate tool-role messages
// ---------------------------------------------------------------------------

/// Translate messages with `role: "tool"` into `role: "user"` messages that
/// contain the tool result in a readable format.
///
/// The Copilot upstream API does not understand the `tool` role, so we must
/// convert these into regular user messages. Each tool message is rewritten
/// as a user message wrapping the result text with context about which tool
/// call it belongs to.
///
/// Messages with other roles are passed through unchanged.
pub fn translate_tool_messages(messages: &mut Vec<Message>) {
    for msg in messages.iter_mut() {
        if msg.role == "tool" {
            let tool_call_id = msg.tool_call_id.take().unwrap_or_default();
            let content = msg.content.as_text();

            msg.role = "user".into();
            msg.content = MessageContent::Text(format!(
                "[Tool Result (call_id: {tool_call_id})]\n{content}"
            ));
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tools::types::{Function, Tool};

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

    #[test]
    fn format_tools_produces_valid_json() {
        let tools = vec![sample_tool(
            "get_weather",
            Some("Get the weather"),
            Some(serde_json::json!({
                "type": "object",
                "properties": { "location": { "type": "string" } },
                "required": ["location"]
            })),
        )];
        let output = format_tools_as_json(&tools);
        let parsed: serde_json::Value = serde_json::from_str(&output).unwrap();
        assert_eq!(parsed["functions"][0]["name"], "get_weather");
    }

    #[test]
    fn build_tool_prompt_contains_instructions() {
        let tools = vec![sample_tool("noop", None, None)];
        let prompt = build_tool_prompt(&tools);
        assert!(prompt.contains("# Available Functions"));
        assert!(prompt.contains("# How to Call Functions"));
        assert!(prompt.contains("function_call"));
    }
}
