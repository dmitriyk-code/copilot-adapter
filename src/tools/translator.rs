use std::collections::HashMap;

use sha2::{Digest, Sha256};

use crate::anthropic::types::{InputSchema, ToolDefinition};
use crate::copilot::types::{OpenAITool, OpenAIToolFunction};

/// Maximum length for tool names in the OpenAI API.
const OPENAI_MAX_TOOL_NAME_LENGTH: usize = 64;

/// Length of the hash suffix appended to truncated tool names.
const TOOL_NAME_HASH_LENGTH: usize = 8;

/// Result of translating Anthropic tools to OpenAI format.
pub struct ToolTranslation {
    /// Translated OpenAI tool definitions.
    pub tools: Vec<OpenAITool>,
    /// Mapping from truncated names back to original names.
    /// Empty if no names were truncated.
    pub name_mapping: HashMap<String, String>,
}

/// Translate Anthropic tool definitions to OpenAI format.
///
/// Handles the 64-character name limit by truncating long names with a hash suffix.
/// The returned `name_mapping` allows restoring original names in responses.
pub fn translate_anthropic_tools_to_openai(tools: &[ToolDefinition]) -> ToolTranslation {
    let mut openai_tools = Vec::new();
    let mut name_mapping = HashMap::new();

    for tool in tools {
        let (name, was_truncated) = truncate_tool_name(&tool.name);

        if was_truncated {
            name_mapping.insert(name.clone(), tool.name.clone());
        }

        openai_tools.push(OpenAITool {
            tool_type: "function".to_string(),
            function: OpenAIToolFunction {
                name,
                description: tool.description.clone(),
                parameters: translate_input_schema(&tool.input_schema),
            },
        });
    }

    ToolTranslation {
        tools: openai_tools,
        name_mapping,
    }
}

/// Truncate a tool name to fit OpenAI's 64-character limit.
///
/// If the name exceeds the limit, it is truncated to 55 characters and
/// suffixed with `_` plus an 8-character hash of the full name.
/// UTF-8 safety: the prefix is sliced at a character boundary to avoid
/// panics on multi-byte tool names.
///
/// Returns `(truncated_name, was_truncated)`.
fn truncate_tool_name(name: &str) -> (String, bool) {
    if name.chars().count() <= OPENAI_MAX_TOOL_NAME_LENGTH {
        return (name.to_string(), false);
    }

    let mut hasher = Sha256::new();
    hasher.update(name.as_bytes());
    let hash = hasher.finalize();
    let hash_hex = hex::encode(&hash[..4]); // 8 hex chars from 4 bytes

    let prefix_len = OPENAI_MAX_TOOL_NAME_LENGTH - 1 - TOOL_NAME_HASH_LENGTH;
    // Take exactly `prefix_len` characters (by character count, not bytes)
    // to respect the OpenAI character limit. Use `char_indices` to find
    // the byte offset after the last included character for safe slicing.
    let safe_byte_end = name
        .char_indices()
        .nth(prefix_len)
        .map(|(i, _)| i)
        .unwrap_or(name.len());
    let truncated = format!("{}_{}", &name[..safe_byte_end], hash_hex);

    (truncated, true)
}

/// Translate Anthropic InputSchema to OpenAI parameters format.
fn translate_input_schema(schema: &InputSchema) -> Option<serde_json::Value> {
    let mut params = serde_json::Map::new();

    params.insert(
        "type".into(),
        serde_json::Value::String(schema.schema_type.clone()),
    );

    if let Some(ref props) = schema.properties {
        params.insert("properties".into(), props.clone());
    }

    if let Some(ref req) = schema.required {
        params.insert(
            "required".into(),
            serde_json::Value::Array(
                req.iter()
                    .map(|s| serde_json::Value::String(s.clone()))
                    .collect(),
            ),
        );
    }

    Some(serde_json::Value::Object(params))
}

/// Restore the original tool name from a potentially truncated name.
pub fn restore_tool_name(name: &str, mapping: &HashMap<String, String>) -> String {
    mapping
        .get(name)
        .cloned()
        .unwrap_or_else(|| name.to_string())
}
