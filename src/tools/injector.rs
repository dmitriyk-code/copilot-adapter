use crate::copilot::types::{Message, MessageContent};
use crate::tools::types::Tool;

// ---------------------------------------------------------------------------
// Tool usage instructions
// ---------------------------------------------------------------------------

/// Instructions appended after tool definitions telling the model how to
/// format tool calls. The parser (`src/tools/parser.rs`) looks for
/// `<function_calls>` XML blocks matching this exact format — keep them in
/// sync.
pub const TOOL_USAGE_INSTRUCTIONS: &str = r#"
In this environment you have access to a set of tools you can use to answer the user's question.

You may call them like this:
<function_calls>
<invoke name="$TOOL_NAME">
<parameter name="$PARAMETER_NAME">$PARAMETER_VALUE</parameter>
...
</invoke>
</function_calls>

Important rules:
- Always wrap tool calls in <function_calls> tags
- Use the name attribute of <invoke> to specify which tool to call
- Use <parameter name="...">value</parameter> to pass arguments
- You can call multiple tools by including multiple <invoke> blocks
- After receiving tool results, continue your response normally
"#;

// ---------------------------------------------------------------------------
// Format tools as XML (LiteLLM/Anthropic Cookbook format)
// ---------------------------------------------------------------------------

/// Escape special XML characters in text content.
fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

/// Format a single parameter from a JSON Schema property into XML.
///
/// Produces:
/// ```xml
/// <parameter>
/// <name>param_name</name>
/// <type>string</type>
/// <description>param description</description>
/// <required>true</required>
/// </parameter>
/// ```
fn format_parameter_xml(name: &str, schema: &serde_json::Value, is_required: bool) -> String {
    let param_type = schema
        .get("type")
        .and_then(|v| v.as_str())
        .unwrap_or("string");

    let description = schema
        .get("description")
        .and_then(|v| v.as_str())
        .unwrap_or("");

    let mut xml = String::new();
    xml.push_str("<parameter>\n");
    xml.push_str(&format!("<name>{}</name>\n", xml_escape(name)));
    xml.push_str(&format!("<type>{}</type>\n", xml_escape(param_type)));
    xml.push_str(&format!(
        "<description>{}</description>\n",
        xml_escape(description)
    ));
    xml.push_str(&format!("<required>{}</required>\n", is_required));
    xml.push_str("</parameter>");
    xml
}

/// Convert a slice of `Tool` definitions into an XML prompt following the
/// LiteLLM/Anthropic Cookbook format.
///
/// Output format:
/// ```xml
/// <tools>
/// <tool_description>
/// <tool_name>function_name</tool_name>
/// <description>description text</description>
/// <parameters>
/// <parameter>
/// <name>param_name</name>
/// <type>string</type>
/// <description>param description</description>
/// <required>true</required>
/// </parameter>
/// </parameters>
/// </tool_description>
/// </tools>
/// ```
pub fn format_tools_as_xml(tools: &[Tool]) -> String {
    let mut xml = String::new();
    xml.push_str("<tools>\n");

    for tool in tools {
        xml.push_str("<tool_description>\n");
        xml.push_str(&format!(
            "<tool_name>{}</tool_name>\n",
            xml_escape(&tool.function.name)
        ));

        if let Some(ref desc) = tool.function.description {
            xml.push_str(&format!(
                "<description>{}</description>\n",
                xml_escape(desc)
            ));
        } else {
            xml.push_str("<description></description>\n");
        }

        if let Some(ref params) = tool.function.parameters {
            let required: Vec<String> = params
                .get("required")
                .and_then(|v| v.as_array())
                .map(|arr| {
                    arr.iter()
                        .filter_map(|v| v.as_str().map(|s| s.to_string()))
                        .collect()
                })
                .unwrap_or_default();

            if let Some(properties) = params.get("properties").and_then(|v| v.as_object()) {
                xml.push_str("<parameters>\n");
                // Sort keys for deterministic output
                let mut keys: Vec<&String> = properties.keys().collect();
                keys.sort();
                for key in keys {
                    let prop_schema = &properties[key];
                    let is_required = required.iter().any(|r| r == key);
                    xml.push_str(&format_parameter_xml(key, prop_schema, is_required));
                    xml.push('\n');
                }
                xml.push_str("</parameters>\n");
            } else {
                xml.push_str("<parameters>\n</parameters>\n");
            }
        } else {
            xml.push_str("<parameters>\n</parameters>\n");
        }

        xml.push_str("</tool_description>\n");
    }

    xml.push_str("</tools>");
    xml
}

// ---------------------------------------------------------------------------
// Inject tools into messages
// ---------------------------------------------------------------------------

/// Inject tool definitions and usage instructions into the message list.
///
/// The function prepends the tool definitions (as formatted XML) and the
/// usage instructions to the **first system message** found in `messages`.
/// If no system message exists, a new one is created and inserted at index 0.
///
/// The original system message content is preserved — the tool block is
/// prepended so it appears before any existing instructions.
///
/// Returns the size of the injected XML prompt (0 if no tools were injected).
pub fn inject_tools_into_messages(
    messages: &mut Vec<Message>,
    tools: &[Tool],
    debug_tools: bool,
) -> usize {
    if tools.is_empty() {
        return 0;
    }

    let tool_prompt = build_tool_prompt(tools);
    let injection_size = tool_prompt.len();

    if debug_tools {
        tracing::info!(
            num_tools = tools.len(),
            tool_names = ?tools.iter().map(|t| &t.function.name).collect::<Vec<_>>(),
            xml_size = injection_size,
            xml_preview = %tool_prompt.chars().take(500).collect::<String>(),
            "DEBUG_TOOLS: Injecting tools into system prompt"
        );
    }

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

    injection_size
}

/// Build the full prompt block that gets injected into the system message.
fn build_tool_prompt(tools: &[Tool]) -> String {
    let xml = format_tools_as_xml(tools);
    format!("# Available Functions\n\n{xml}\n{TOOL_USAGE_INSTRUCTIONS}")
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
pub fn translate_tool_messages(messages: &mut [Message]) {
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

    /// Helper shared by tests that need a `Tool` with optional description
    /// and parameters.
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

    // -- XML formatting ---------------------------------------------------

    #[test]
    fn format_tools_with_multiple_params() {
        let tools = vec![sample_tool(
            "search",
            Some("Search for items"),
            Some(serde_json::json!({
                "type": "object",
                "properties": {
                    "query": { "type": "string", "description": "Search query" },
                    "limit": { "type": "number", "description": "Max results" },
                    "verbose": { "type": "boolean", "description": "Verbose output" }
                },
                "required": ["query"]
            })),
        )];
        let output = format_tools_as_xml(&tools);

        assert!(output.contains("<name>query</name>"));
        assert!(output.contains("<name>limit</name>"));
        assert!(output.contains("<name>verbose</name>"));
        assert!(output.contains("<type>string</type>"));
        assert!(output.contains("<type>number</type>"));
        assert!(output.contains("<type>boolean</type>"));

        let query_idx = output.find("<name>query</name>").unwrap();
        assert!(output[query_idx..].contains("<required>true</required>"));

        let limit_idx = output.find("<name>limit</name>").unwrap();
        assert!(output[limit_idx..].contains("<required>false</required>"));
    }

    #[test]
    fn format_tools_with_nested_params() {
        let tools = vec![sample_tool(
            "create",
            Some("Create an item"),
            Some(serde_json::json!({
                "type": "object",
                "properties": {
                    "data": { "type": "object", "description": "Nested object data" },
                    "tags": { "type": "array", "description": "List of tags" }
                },
                "required": ["data"]
            })),
        )];
        let output = format_tools_as_xml(&tools);
        assert!(output.contains("<type>object</type>"));
        assert!(output.contains("<type>array</type>"));
        assert!(output.contains("<description>Nested object data</description>"));
        assert!(output.contains("<description>List of tags</description>"));
    }

    #[test]
    fn format_tools_escapes_special_chars() {
        let tools = vec![sample_tool(
            "compare",
            Some("Check if a < b & c > d"),
            Some(serde_json::json!({
                "type": "object",
                "properties": {
                    "expr": { "type": "string", "description": "Expression like <a> & <b>" }
                },
                "required": ["expr"]
            })),
        )];
        let output = format_tools_as_xml(&tools);
        assert!(output.contains("<description>Check if a &lt; b &amp; c &gt; d</description>"));
        assert!(
            output.contains("<description>Expression like &lt;a&gt; &amp; &lt;b&gt;</description>")
        );
        assert!(output.contains("<tool_name>compare</tool_name>"));
    }

    #[test]
    fn format_tools_without_description() {
        let tools = vec![sample_tool("noop", None, None)];
        let output = format_tools_as_xml(&tools);
        assert!(output.contains("<tool_name>noop</tool_name>"));
        assert!(output.contains("<description></description>"));
    }

    #[test]
    fn format_tools_param_without_description() {
        let tools = vec![sample_tool(
            "test",
            Some("Test tool"),
            Some(serde_json::json!({
                "type": "object",
                "properties": {
                    "value": { "type": "string" }
                },
                "required": []
            })),
        )];
        let output = format_tools_as_xml(&tools);
        assert!(output.contains("<name>value</name>"));
        assert!(output.contains("<type>string</type>"));
        assert!(output.contains("<description></description>"));
        assert!(output.contains("<required>false</required>"));
    }

    // -- build_tool_prompt ------------------------------------------------

    #[test]
    fn build_tool_prompt_contains_instructions() {
        let tools = vec![sample_tool("noop", None, None)];
        let prompt = build_tool_prompt(&tools);
        assert!(prompt.contains("# Available Functions"));
        assert!(prompt.contains("<function_calls>"));
        // Invocation format must use attribute syntax matching the parser
        assert!(prompt.contains(r#"invoke name="#));
        assert!(prompt.contains(r#"parameter name="#));
        assert!(prompt.contains("<tools>"));
    }

    // -- xml_escape -------------------------------------------------------

    #[test]
    fn xml_escape_handles_all_special_chars() {
        assert_eq!(xml_escape("a < b"), "a &lt; b");
        assert_eq!(xml_escape("a > b"), "a &gt; b");
        assert_eq!(xml_escape("a & b"), "a &amp; b");
        assert_eq!(xml_escape("<>&"), "&lt;&gt;&amp;");
        assert_eq!(xml_escape("no special"), "no special");
        assert_eq!(xml_escape(""), "");
    }
}
