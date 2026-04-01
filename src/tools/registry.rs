//! Tool registry for schema-aware parameter type coercion.
//!
//! When the XML fallback path parses tool calls from model-generated text,
//! all parameter values are initially strings. This module provides a
//! [`ToolRegistry`] that maps tool names and parameter names to their
//! JSON Schema types, enabling automatic coercion of strings to the
//! correct JSON types (numbers, booleans, objects, arrays).
//!
//! # Usage
//!
//! ```ignore
//! let registry = ToolRegistry::from_tools(&request.tools);
//! let calls = parse_tool_calls(content, Some(&registry), false);
//! ```

use std::collections::HashMap;

use crate::anthropic::types::ToolDefinition;

/// Registry for looking up tool parameter types from schemas.
#[derive(Debug, Clone, Default)]
pub struct ToolRegistry {
    /// Map from tool name to parameter schemas.
    tools: HashMap<String, ToolSchema>,
}

/// Schema information for a single tool's parameters.
#[derive(Debug, Clone)]
struct ToolSchema {
    /// Map from parameter name to its JSON Schema type.
    params: HashMap<String, ParamType>,
}

/// JSON Schema parameter types used for value coercion.
#[derive(Debug, Clone, PartialEq)]
pub enum ParamType {
    String,
    Number,
    Integer,
    Boolean,
    Object,
    Array,
    Null,
}

impl ToolRegistry {
    /// Build a registry from Anthropic tool definitions.
    ///
    /// Extracts the `type` field from each property in the tool's
    /// `input_schema.properties` object.
    pub fn from_tools(tools: &[ToolDefinition]) -> Self {
        let mut registry = ToolRegistry::default();

        for tool in tools {
            let mut params = HashMap::new();

            if let Some(properties) = &tool.input_schema.properties {
                if let Some(props_obj) = properties.as_object() {
                    for (param_name, param_schema) in props_obj {
                        if let Some(param_type) = extract_param_type(param_schema) {
                            params.insert(param_name.clone(), param_type);
                        }
                    }
                }
            }

            registry
                .tools
                .insert(tool.name.clone(), ToolSchema { params });
        }

        registry
    }

    /// Look up the expected type for a parameter.
    ///
    /// Returns `None` if the tool or parameter is not found.
    pub fn get_param_type(&self, tool_name: &str, param_name: &str) -> Option<&ParamType> {
        self.tools
            .get(tool_name)
            .and_then(|schema| schema.params.get(param_name))
    }
}

/// Extract the parameter type from a JSON Schema property.
fn extract_param_type(schema: &serde_json::Value) -> Option<ParamType> {
    schema
        .get("type")
        .and_then(|t| t.as_str())
        .map(|s| match s {
            "string" => ParamType::String,
            "number" => ParamType::Number,
            "integer" => ParamType::Integer,
            "boolean" => ParamType::Boolean,
            "object" => ParamType::Object,
            "array" => ParamType::Array,
            "null" => ParamType::Null,
            _ => ParamType::String, // Default to string for unknown types
        })
}

/// Parse a string value according to the expected type.
///
/// Falls back to string if parsing fails or type is unknown.
pub fn parse_value_with_type(value: &str, param_type: &ParamType) -> serde_json::Value {
    match param_type {
        ParamType::Number => value
            .parse::<f64>()
            .map(serde_json::Value::from)
            .unwrap_or_else(|_| serde_json::Value::String(value.to_string())),
        ParamType::Integer => value
            .parse::<i64>()
            .map(serde_json::Value::from)
            .unwrap_or_else(|_| serde_json::Value::String(value.to_string())),
        ParamType::Boolean => match value.to_lowercase().as_str() {
            "true" => serde_json::Value::Bool(true),
            "false" => serde_json::Value::Bool(false),
            _ => serde_json::Value::String(value.to_string()),
        },
        ParamType::Null => {
            if value.to_lowercase() == "null" {
                serde_json::Value::Null
            } else {
                serde_json::Value::String(value.to_string())
            }
        }
        ParamType::Object => {
            // Parse as a JSON object specifically — reject other JSON types
            serde_json::from_str::<serde_json::Map<String, serde_json::Value>>(value)
                .map(serde_json::Value::Object)
                .unwrap_or_else(|_| serde_json::Value::String(value.to_string()))
        }
        ParamType::Array => {
            // Parse as a JSON array specifically — reject other JSON types
            serde_json::from_str::<Vec<serde_json::Value>>(value)
                .map(serde_json::Value::Array)
                .unwrap_or_else(|_| serde_json::Value::String(value.to_string()))
        }
        ParamType::String => serde_json::Value::String(value.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::anthropic::types::{InputSchema, ToolDefinition};

    fn make_tool(name: &str, properties: serde_json::Value) -> ToolDefinition {
        ToolDefinition {
            name: name.to_string(),
            description: Some("test tool".to_string()),
            input_schema: InputSchema {
                schema_type: "object".to_string(),
                properties: Some(properties),
                required: None,
            },
        }
    }

    // -- ToolRegistry construction ----------------------------------------

    #[test]
    fn registry_from_tools_extracts_types() {
        let tools = vec![make_tool(
            "search",
            serde_json::json!({
                "query": {"type": "string"},
                "limit": {"type": "integer"},
                "fuzzy": {"type": "boolean"}
            }),
        )];

        let registry = ToolRegistry::from_tools(&tools);

        assert_eq!(
            registry.get_param_type("search", "query"),
            Some(&ParamType::String)
        );
        assert_eq!(
            registry.get_param_type("search", "limit"),
            Some(&ParamType::Integer)
        );
        assert_eq!(
            registry.get_param_type("search", "fuzzy"),
            Some(&ParamType::Boolean)
        );
    }

    #[test]
    fn registry_returns_none_for_unknown_tool() {
        let registry = ToolRegistry::from_tools(&[]);
        assert_eq!(registry.get_param_type("nonexistent", "param"), None);
    }

    #[test]
    fn registry_returns_none_for_unknown_param() {
        let tools = vec![make_tool(
            "search",
            serde_json::json!({"query": {"type": "string"}}),
        )];
        let registry = ToolRegistry::from_tools(&tools);
        assert_eq!(registry.get_param_type("search", "unknown"), None);
    }

    #[test]
    fn registry_handles_no_properties() {
        let tool = ToolDefinition {
            name: "empty".to_string(),
            description: None,
            input_schema: InputSchema {
                schema_type: "object".to_string(),
                properties: None,
                required: None,
            },
        };
        let registry = ToolRegistry::from_tools(&[tool]);
        assert_eq!(registry.get_param_type("empty", "anything"), None);
    }

    #[test]
    fn registry_handles_missing_type_field() {
        let tools = vec![make_tool(
            "tool",
            serde_json::json!({
                "desc_only": {"description": "no type here"}
            }),
        )];
        let registry = ToolRegistry::from_tools(&tools);
        assert_eq!(registry.get_param_type("tool", "desc_only"), None);
    }

    #[test]
    fn registry_handles_all_types() {
        let tools = vec![make_tool(
            "all_types",
            serde_json::json!({
                "s": {"type": "string"},
                "n": {"type": "number"},
                "i": {"type": "integer"},
                "b": {"type": "boolean"},
                "o": {"type": "object"},
                "a": {"type": "array"},
                "nil": {"type": "null"},
                "custom": {"type": "foobar"}
            }),
        )];

        let registry = ToolRegistry::from_tools(&tools);
        assert_eq!(
            registry.get_param_type("all_types", "s"),
            Some(&ParamType::String)
        );
        assert_eq!(
            registry.get_param_type("all_types", "n"),
            Some(&ParamType::Number)
        );
        assert_eq!(
            registry.get_param_type("all_types", "i"),
            Some(&ParamType::Integer)
        );
        assert_eq!(
            registry.get_param_type("all_types", "b"),
            Some(&ParamType::Boolean)
        );
        assert_eq!(
            registry.get_param_type("all_types", "o"),
            Some(&ParamType::Object)
        );
        assert_eq!(
            registry.get_param_type("all_types", "a"),
            Some(&ParamType::Array)
        );
        assert_eq!(
            registry.get_param_type("all_types", "nil"),
            Some(&ParamType::Null)
        );
        // Unknown type defaults to String
        assert_eq!(
            registry.get_param_type("all_types", "custom"),
            Some(&ParamType::String)
        );
    }

    // -- parse_value_with_type --------------------------------------------

    #[test]
    fn parse_number_valid() {
        let val = parse_value_with_type("42.5", &ParamType::Number);
        assert_eq!(val, serde_json::json!(42.5));
    }

    #[test]
    fn parse_number_integer_value() {
        let val = parse_value_with_type("100", &ParamType::Number);
        assert_eq!(val, serde_json::json!(100.0));
    }

    #[test]
    fn parse_number_invalid_falls_back() {
        let val = parse_value_with_type("not_a_number", &ParamType::Number);
        assert_eq!(val, serde_json::json!("not_a_number"));
    }

    #[test]
    fn parse_integer_valid() {
        let val = parse_value_with_type("42", &ParamType::Integer);
        assert_eq!(val, serde_json::json!(42));
    }

    #[test]
    fn parse_integer_negative() {
        let val = parse_value_with_type("-7", &ParamType::Integer);
        assert_eq!(val, serde_json::json!(-7));
    }

    #[test]
    fn parse_integer_float_falls_back() {
        let val = parse_value_with_type("3.14", &ParamType::Integer);
        assert_eq!(val, serde_json::json!("3.14"));
    }

    #[test]
    fn parse_boolean_true() {
        let val = parse_value_with_type("true", &ParamType::Boolean);
        assert_eq!(val, serde_json::json!(true));
    }

    #[test]
    fn parse_boolean_false() {
        let val = parse_value_with_type("false", &ParamType::Boolean);
        assert_eq!(val, serde_json::json!(false));
    }

    #[test]
    fn parse_boolean_case_insensitive() {
        assert_eq!(
            parse_value_with_type("TRUE", &ParamType::Boolean),
            serde_json::json!(true)
        );
        assert_eq!(
            parse_value_with_type("False", &ParamType::Boolean),
            serde_json::json!(false)
        );
    }

    #[test]
    fn parse_boolean_invalid_falls_back() {
        let val = parse_value_with_type("yes", &ParamType::Boolean);
        assert_eq!(val, serde_json::json!("yes"));
    }

    #[test]
    fn parse_null_valid() {
        let val = parse_value_with_type("null", &ParamType::Null);
        assert_eq!(val, serde_json::Value::Null);
    }

    #[test]
    fn parse_null_case_insensitive() {
        let val = parse_value_with_type("NULL", &ParamType::Null);
        assert_eq!(val, serde_json::Value::Null);
    }

    #[test]
    fn parse_null_invalid_falls_back() {
        let val = parse_value_with_type("something", &ParamType::Null);
        assert_eq!(val, serde_json::json!("something"));
    }

    #[test]
    fn parse_object_valid() {
        let val = parse_value_with_type(r#"{"key": "value"}"#, &ParamType::Object);
        assert_eq!(val, serde_json::json!({"key": "value"}));
    }

    #[test]
    fn parse_object_nested() {
        let val = parse_value_with_type(r#"{"a": {"b": 1}}"#, &ParamType::Object);
        assert_eq!(val, serde_json::json!({"a": {"b": 1}}));
    }

    #[test]
    fn parse_object_invalid_falls_back() {
        let val = parse_value_with_type("not json", &ParamType::Object);
        assert_eq!(val, serde_json::json!("not json"));
    }

    #[test]
    fn parse_array_valid() {
        let val = parse_value_with_type(r#"[1, 2, 3]"#, &ParamType::Array);
        assert_eq!(val, serde_json::json!([1, 2, 3]));
    }

    #[test]
    fn parse_array_of_strings() {
        let val = parse_value_with_type(r#"["a", "b"]"#, &ParamType::Array);
        assert_eq!(val, serde_json::json!(["a", "b"]));
    }

    #[test]
    fn parse_array_invalid_falls_back() {
        let val = parse_value_with_type("not an array", &ParamType::Array);
        assert_eq!(val, serde_json::json!("not an array"));
    }

    #[test]
    fn parse_string_always_returns_string() {
        let val = parse_value_with_type("42", &ParamType::String);
        assert_eq!(val, serde_json::json!("42"));
    }
}
