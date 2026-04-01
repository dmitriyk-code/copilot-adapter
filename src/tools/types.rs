use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// OpenAI Tool definition types (request-side)
// ---------------------------------------------------------------------------

/// An OpenAI-format tool definition.
///
/// Schema: `{ type: "function", function: { name, description?, parameters } }`
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Tool {
    #[serde(rename = "type")]
    pub tool_type: String,
    pub function: Function,
}

/// Function definition within a tool.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Function {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parameters: Option<FunctionParameters>,
}

/// JSON Schema object describing the function's parameters.
/// Uses `serde_json::Value` for maximum flexibility since JSON Schema
/// can have arbitrary structure.
pub type FunctionParameters = serde_json::Value;

// ---------------------------------------------------------------------------
// OpenAI ToolCall types (response-side)
// ---------------------------------------------------------------------------

/// A tool call returned by the model in a response or streaming chunk.
///
/// Schema: `{ id: "call_xxx", type: "function", function: { name, arguments } }`
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCall {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    #[serde(rename = "type")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub call_type: Option<String>,
    pub function: FunctionCall,
}

/// The function invocation within a tool call.
///
/// Note: `arguments` is a JSON-encoded string, not a parsed object.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FunctionCall {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub arguments: Option<String>,
}
