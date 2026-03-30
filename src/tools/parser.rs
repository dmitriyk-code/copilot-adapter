//! Tool call parser for model-generated text content.
//!
//! This module extracts tool calls from assistant responses using XML format.
//!
//! Two XML dialects are supported:
//!
//! 1. **Tag-based** —
//!    ```xml
//!    <function_calls>
//!      <invoke>
//!        <tool_name>ToolName</tool_name>
//!        <parameters>
//!          <param_name>value</param_name>
//!        </parameters>
//!      </invoke>
//!    </function_calls>
//!    ```
//!
//! 2. **Attribute-based** (emitted by injector instructions) —
//!    ```xml
//!    <function_calls>
//!      <invoke name="ToolName">
//!        <parameter name="key">value</parameter>
//!      </invoke>
//!    </function_calls>
//!    ```
//!
//! Both dialects are tried when parsing. Standalone `<invoke>` blocks
//! (without a `<function_calls>` wrapper) are accepted as a fallback.
//!
//! The public API consists of two functions:
//! - [`parse_tool_calls`] — extract `ToolCall` structs from text
//! - [`strip_tool_calls`] — remove tool call markup, leaving surrounding prose

use once_cell::sync::Lazy;
use regex::Regex;
use uuid::Uuid;

use crate::tools::types::{FunctionCall, ToolCall};

// ---------------------------------------------------------------------------
// Regex patterns — compiled once
// ---------------------------------------------------------------------------

/// Collapses runs of 3+ newlines down to 2 (one blank line).
static COLLAPSE_NEWLINES: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"\n{3,}").expect("collapse regex should compile")
});

/// Matches `<function_calls>...</function_calls>` blocks for stripping.
static STRIP_FUNCTION_CALLS: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"(?s)<function_calls>.*?</function_calls>")
        .expect("function_calls strip regex should compile")
});

/// Matches standalone `<invoke...>...</invoke>` blocks for stripping.
static STRIP_INVOKE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"(?s)<invoke[^>]*>.*?</invoke>")
        .expect("invoke strip regex should compile")
});

/// Matches `<invoke name="...">...</invoke>` blocks (attribute-based format).
///
/// The name capture uses `[^"]*` (zero-or-more) so that empty names are
/// captured and rejected by the guard rather than silently ignored.
static XML_ATTR_INVOKE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r#"(?s)<invoke\s+name="([^"]*)">(.*?)</invoke>"#)
        .expect("xml attr invoke regex should compile")
});

/// Matches `<parameter name="...">...</parameter>` (attribute-based format).
///
/// Uses a lazy `(.*?)` capture so that parameter values containing `<`
/// (e.g. comparison operators, HTML snippets) are not truncated.
static XML_PARAMETER: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r#"(?s)<parameter\s+name="([^"]+)">(.*?)</parameter>"#)
        .expect("xml parameter regex should compile")
});

// ---------------------------------------------------------------------------
// XML extraction helpers
// ---------------------------------------------------------------------------

/// Extract content between XML tags.
fn extract_between_tags(tag: &str, content: &str) -> Vec<String> {
    let pattern = format!(
        r"(?s)<{}>(.+?)</{}>",
        regex::escape(tag),
        regex::escape(tag)
    );
    let regex = Regex::new(&pattern).expect("tag extraction regex should compile");
    regex
        .captures_iter(content)
        .filter_map(|cap| cap.get(1).map(|m| m.as_str().to_string()))
        .collect()
}

/// Check if a tag exists in the content.
///
/// Delegates to [`extract_between_tags`] to avoid duplicating the regex.
fn contains_tag(tag: &str, content: &str) -> bool {
    !extract_between_tags(tag, content).is_empty()
}

/// Matches opening tags like `<param_name>` in tag-based XML parameters.
///
/// Compiled once and reused across all calls to [`parse_xml_params`].
static OPEN_TAG: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"<([a-zA-Z_][a-zA-Z0-9_]*)>").expect("open tag regex should compile")
});

/// Parse XML parameters in tag-based format into a JSON object.
///
/// Input: `<file_path>/src/main.rs</file_path><limit>100</limit>`
/// Output: `{"file_path": "/src/main.rs", "limit": "100"}`
fn parse_xml_params(params_content: &str) -> serde_json::Value {
    let mut params = serde_json::Map::new();

    // Match opening tags and manually find their closing counterpart.
    // The `regex` crate does not support backreferences, so we locate
    // `</tag_name>` with a simple string search instead.
    for cap in OPEN_TAG.captures_iter(params_content) {
        let full_match = cap.get(0).unwrap();
        let name = cap.get(1).unwrap().as_str();
        let closing_tag = format!("</{name}>");

        let after_open = full_match.end();
        if let Some(close_pos) = params_content[after_open..].find(&closing_tag) {
            let value = &params_content[after_open..after_open + close_pos];
            params.insert(
                name.to_string(),
                serde_json::Value::String(value.trim().to_string()),
            );
        }
    }

    serde_json::Value::Object(params)
}

/// Parse attribute-based XML parameters into a JSON object.
///
/// Input: `<parameter name="file_path">/src/main.rs</parameter>`
/// Output: `{"file_path": "/src/main.rs"}`
fn parse_attribute_params(invoke_body: &str) -> serde_json::Value {
    let mut params = serde_json::Map::new();

    for param_cap in XML_PARAMETER.captures_iter(invoke_body) {
        let param_name = param_cap.get(1).unwrap().as_str();
        let param_value = param_cap.get(2).unwrap().as_str().trim();
        params.insert(
            param_name.to_string(),
            serde_json::Value::String(param_value.to_string()),
        );
    }

    serde_json::Value::Object(params)
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Parse tool calls from model-generated text content.
///
/// Primary: Look for `<function_calls>` wrapper blocks containing `<invoke>`
/// elements in either tag-based or attribute-based format.
///
/// Fallback: Look for standalone `<invoke>` blocks.
///
/// Each extracted tool call is assigned a unique `call_xxx` ID.
/// Multiple tool calls are returned in document order.
///
/// Invalid or malformed content is silently skipped.
///
/// When `debug_tools` is `true`, additional INFO-level logs are emitted
/// showing parsing results and diagnostic details.
pub fn parse_tool_calls(content: &str, debug_tools: bool) -> Vec<ToolCall> {
    // Primary: Look for <function_calls> wrapper
    if contains_tag("function_calls", content) {
        let fc_content = extract_between_tags("function_calls", content);
        let mut calls = Vec::new();
        for fc in fc_content {
            calls.extend(parse_invokes(&fc));
        }
        if !calls.is_empty() {
            tracing::debug!(
                num_calls = calls.len(),
                "Parsed tool calls from <function_calls> blocks"
            );
            if debug_tools {
                tracing::info!(
                    num_calls = calls.len(),
                    tool_names = ?calls.iter().map(|tc| &tc.function.name).collect::<Vec<_>>(),
                    "DEBUG_TOOLS: Successfully parsed tool calls from <function_calls> blocks"
                );
            }
            return calls;
        }
    }

    // Fallback: Look for standalone <invoke> blocks
    let calls = parse_invokes(content);
    if !calls.is_empty() {
        tracing::debug!(
            num_calls = calls.len(),
            "Parsed tool calls from standalone <invoke> blocks"
        );
        if debug_tools {
            tracing::info!(
                num_calls = calls.len(),
                tool_names = ?calls.iter().map(|tc| &tc.function.name).collect::<Vec<_>>(),
                "DEBUG_TOOLS: Successfully parsed tool calls from standalone <invoke> blocks"
            );
        }
        return calls;
    }

    // If no tool calls found but content looks like it might have them, log for debugging
    if content.contains("<invoke")
        || content.contains("<tool")
        || content.contains("function_call")
        || content.contains("<function")
    {
        tracing::warn!(
            content_preview = %content.chars().take(500).collect::<String>(),
            "Content contains tool-like patterns but no valid tool calls were parsed"
        );
    }

    if debug_tools {
        tracing::info!(
            content_length = content.len(),
            has_invoke = content.contains("<invoke"),
            has_tool_name = contains_tag("tool_name", content),
            has_function_calls = contains_tag("function_calls", content),
            content_preview = %content.chars().take(300).collect::<String>(),
            "DEBUG_TOOLS: No tool calls parsed from response"
        );
    }

    calls
}

// ---------------------------------------------------------------------------
// Internal parsers
// ---------------------------------------------------------------------------

/// Parse `<invoke>` blocks from content, supporting both tag-based and
/// attribute-based formats.
///
/// **Ordering:** Tag-based results are collected first, then attribute-based.
/// If both formats appear in the same block the output order may differ from
/// document order. In practice models use one format consistently so this
/// is not an issue.
fn parse_invokes(content: &str) -> Vec<ToolCall> {
    let mut calls = Vec::new();

    // Tag-based: <invoke><tool_name>...</tool_name>...</invoke>
    for invoke_content in extract_between_tags("invoke", content) {
        if let Some(tc) = try_parse_tag_invoke(&invoke_content) {
            calls.push(tc);
        }
    }

    // Attribute-based: <invoke name="...">...</invoke>
    for cap in XML_ATTR_INVOKE.captures_iter(content) {
        let name = cap.get(1).unwrap().as_str();
        let invoke_body = cap.get(2).unwrap().as_str();
        if let Some(tc) = try_parse_attr_invoke(name, invoke_body) {
            calls.push(tc);
        }
    }

    calls
}

/// Try to parse a tag-based `<invoke>` block.
///
/// Expected inner content:
/// ```xml
/// <tool_name>ToolName</tool_name>
/// <parameters>
///   <param_name>value</param_name>
/// </parameters>
/// ```
fn try_parse_tag_invoke(invoke_content: &str) -> Option<ToolCall> {
    let tool_name = extract_between_tags("tool_name", invoke_content)
        .first()
        .map(|s| s.trim().to_string())?;

    if tool_name.is_empty() {
        return None;
    }

    let params = extract_between_tags("parameters", invoke_content)
        .first()
        .map(|s| parse_xml_params(s))
        .unwrap_or(serde_json::Value::Object(serde_json::Map::new()));

    Some(ToolCall {
        id: Some(generate_call_id()),
        call_type: Some("function".to_string()),
        function: FunctionCall {
            name: Some(tool_name),
            arguments: Some(params.to_string()),
        },
    })
}

/// Try to parse an attribute-based `<invoke name="...">` block.
///
/// `name` is the tool name from the `name` attribute.
/// `invoke_body` is the inner content of the `<invoke>` element.
///
/// Returns `None` if the name is empty.
fn try_parse_attr_invoke(name: &str, invoke_body: &str) -> Option<ToolCall> {
    if name.is_empty() {
        return None;
    }

    let params = parse_attribute_params(invoke_body);

    let arguments = if params.as_object().map_or(true, |m| m.is_empty()) {
        Some("{}".to_string())
    } else {
        Some(params.to_string())
    };

    Some(ToolCall {
        id: Some(generate_call_id()),
        call_type: Some("function".to_string()),
        function: FunctionCall {
            name: Some(name.to_string()),
            arguments,
        },
    })
}

/// Strip tool call XML from the content, returning the cleaned text.
///
/// Removes `<function_calls>` blocks and standalone `<invoke>` blocks,
/// leaving surrounding prose intact. Extra blank lines left by removal
/// are collapsed.
///
/// **Note:** The result is always trimmed of leading/trailing whitespace
/// regardless of whether any tool calls were removed.
pub fn strip_tool_calls(content: &str) -> String {
    let mut result = content.to_string();

    // Remove <function_calls>...</function_calls> blocks
    result = STRIP_FUNCTION_CALLS
        .replace_all(&result, "")
        .to_string();

    // Remove standalone <invoke>...</invoke> blocks
    result = STRIP_INVOKE.replace_all(&result, "").to_string();

    // Collapse runs of 3+ newlines down to 2 (one blank line)
    result = COLLAPSE_NEWLINES.replace_all(&result, "\n\n").to_string();

    result.trim().to_string()
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

/// Generate a unique tool call ID in the format `call_<short-hex>`.
fn generate_call_id() -> String {
    let uuid = Uuid::new_v4();
    // Use first 12 hex chars of the UUID for a compact but unique ID
    let hex = uuid.as_simple().to_string();
    format!("call_{}", &hex[..12])
}

// ---------------------------------------------------------------------------
// Unit tests (inline)
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // -- generate_call_id -------------------------------------------------

    #[test]
    fn generate_call_id_has_correct_prefix() {
        let id = generate_call_id();
        assert!(id.starts_with("call_"));
        assert_eq!(id.len(), 17); // "call_" (5) + 12 hex chars
    }

    #[test]
    fn generate_call_id_is_unique() {
        let id1 = generate_call_id();
        let id2 = generate_call_id();
        assert_ne!(id1, id2);
    }

    // -- extract_between_tags ---------------------------------------------

    #[test]
    fn extract_between_tags_simple() {
        let content = "<tool_name>Read</tool_name>";
        let results = extract_between_tags("tool_name", content);
        assert_eq!(results, vec!["Read"]);
    }

    #[test]
    fn extract_between_tags_multiline() {
        let content = "<parameters>\n<file_path>/a.rs</file_path>\n</parameters>";
        let results = extract_between_tags("parameters", content);
        assert_eq!(results.len(), 1);
        assert!(results[0].contains("<file_path>"));
    }

    #[test]
    fn extract_between_tags_multiple() {
        let content = "<invoke>first</invoke> text <invoke>second</invoke>";
        let results = extract_between_tags("invoke", content);
        assert_eq!(results, vec!["first", "second"]);
    }

    #[test]
    fn extract_between_tags_empty_returns_none() {
        let content = "<invoke></invoke>";
        let results = extract_between_tags("invoke", content);
        assert!(results.is_empty());
    }

    // -- contains_tag -----------------------------------------------------

    #[test]
    fn contains_tag_found() {
        assert!(contains_tag(
            "function_calls",
            "<function_calls>body</function_calls>"
        ));
    }

    #[test]
    fn contains_tag_not_found() {
        assert!(!contains_tag("function_calls", "no xml here"));
    }

    #[test]
    fn contains_tag_empty_body() {
        assert!(!contains_tag(
            "function_calls",
            "<function_calls></function_calls>"
        ));
    }

    // -- parse_xml_params -------------------------------------------------

    #[test]
    fn parse_xml_params_simple() {
        let content = "<file_path>/src/main.rs</file_path><limit>100</limit>";
        let params = parse_xml_params(content);
        assert_eq!(params["file_path"], "/src/main.rs");
        assert_eq!(params["limit"], "100");
    }

    #[test]
    fn parse_xml_params_with_whitespace() {
        let content = "<name>  hello world  </name>";
        let params = parse_xml_params(content);
        assert_eq!(params["name"], "hello world");
    }

    #[test]
    fn parse_xml_params_empty() {
        let params = parse_xml_params("");
        assert!(params.as_object().unwrap().is_empty());
    }

    // -- try_parse_tag_invoke ---------------------------------------------

    #[test]
    fn try_parse_tag_invoke_basic() {
        let content = "<tool_name>Read</tool_name>\n<parameters>\n<file_path>/a.rs</file_path>\n</parameters>";
        let tc = try_parse_tag_invoke(content).unwrap();
        assert_eq!(tc.function.name, Some("Read".to_string()));
        let args: serde_json::Value =
            serde_json::from_str(tc.function.arguments.as_ref().unwrap()).unwrap();
        assert_eq!(args["file_path"], "/a.rs");
    }

    #[test]
    fn try_parse_tag_invoke_no_params() {
        let content = "<tool_name>NoOp</tool_name>";
        let tc = try_parse_tag_invoke(content).unwrap();
        assert_eq!(tc.function.name, Some("NoOp".to_string()));
        let args: serde_json::Value =
            serde_json::from_str(tc.function.arguments.as_ref().unwrap()).unwrap();
        assert!(args.as_object().unwrap().is_empty());
    }

    #[test]
    fn try_parse_tag_invoke_empty_name() {
        let content = "<tool_name></tool_name>";
        assert!(try_parse_tag_invoke(content).is_none());
    }

    // -- try_parse_attr_invoke --------------------------------------------

    #[test]
    fn try_parse_attr_invoke_basic() {
        let body = r#"<parameter name="command">ls</parameter>"#;
        let tc = try_parse_attr_invoke("Bash", body).unwrap();
        assert_eq!(tc.function.name, Some("Bash".to_string()));
        let args: serde_json::Value =
            serde_json::from_str(tc.function.arguments.as_ref().unwrap()).unwrap();
        assert_eq!(args["command"], "ls");
    }

    #[test]
    fn try_parse_attr_invoke_empty_name() {
        assert!(try_parse_attr_invoke("", "").is_none());
    }
}
