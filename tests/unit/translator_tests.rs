use std::collections::HashMap;

use copilot_adapter::anthropic::types::{InputSchema, ToolDefinition};
use copilot_adapter::tools::translator::{restore_tool_name, translate_anthropic_tools_to_openai};

// ---------------------------------------------------------------------------
// Basic tool translation
// ---------------------------------------------------------------------------

#[test]
fn translates_single_tool_to_openai_format() {
    let tools = vec![ToolDefinition {
        name: "bash".to_string(),
        description: Some("Execute a bash command".to_string()),
        input_schema: InputSchema {
            schema_type: "object".to_string(),
            properties: Some(serde_json::json!({
                "command": { "type": "string", "description": "The command to run" }
            })),
            required: Some(vec!["command".to_string()]),
        },
    }];

    let result = translate_anthropic_tools_to_openai(&tools);

    assert_eq!(result.tools.len(), 1);
    let tool = &result.tools[0];
    assert_eq!(tool.tool_type, "function");
    assert_eq!(tool.function.name, "bash");
    assert_eq!(
        tool.function.description,
        Some("Execute a bash command".to_string())
    );
    assert!(result.name_mapping.is_empty());
}

#[test]
fn translates_multiple_tools() {
    let tools = vec![
        ToolDefinition {
            name: "bash".to_string(),
            description: Some("Run bash".to_string()),
            input_schema: InputSchema {
                schema_type: "object".to_string(),
                properties: Some(serde_json::json!({
                    "command": { "type": "string" }
                })),
                required: Some(vec!["command".to_string()]),
            },
        },
        ToolDefinition {
            name: "read_file".to_string(),
            description: Some("Read a file".to_string()),
            input_schema: InputSchema {
                schema_type: "object".to_string(),
                properties: Some(serde_json::json!({
                    "path": { "type": "string" }
                })),
                required: Some(vec!["path".to_string()]),
            },
        },
    ];

    let result = translate_anthropic_tools_to_openai(&tools);

    assert_eq!(result.tools.len(), 2);
    assert_eq!(result.tools[0].function.name, "bash");
    assert_eq!(result.tools[1].function.name, "read_file");
}

#[test]
fn translates_tool_without_description() {
    let tools = vec![ToolDefinition {
        name: "noop".to_string(),
        description: None,
        input_schema: InputSchema {
            schema_type: "object".to_string(),
            properties: None,
            required: None,
        },
    }];

    let result = translate_anthropic_tools_to_openai(&tools);

    assert_eq!(result.tools.len(), 1);
    assert!(result.tools[0].function.description.is_none());
}

#[test]
fn translates_empty_tool_list() {
    let tools: Vec<ToolDefinition> = vec![];
    let result = translate_anthropic_tools_to_openai(&tools);
    assert!(result.tools.is_empty());
    assert!(result.name_mapping.is_empty());
}

// ---------------------------------------------------------------------------
// Tool name truncation
// ---------------------------------------------------------------------------

#[test]
fn short_name_not_truncated() {
    let tools = vec![ToolDefinition {
        name: "bash".to_string(),
        description: None,
        input_schema: InputSchema {
            schema_type: "object".to_string(),
            properties: None,
            required: None,
        },
    }];

    let result = translate_anthropic_tools_to_openai(&tools);

    assert_eq!(result.tools[0].function.name, "bash");
    assert!(result.name_mapping.is_empty());
}

#[test]
fn name_at_exactly_64_chars_not_truncated() {
    let name = "a".repeat(64);
    let tools = vec![ToolDefinition {
        name: name.clone(),
        description: None,
        input_schema: InputSchema {
            schema_type: "object".to_string(),
            properties: None,
            required: None,
        },
    }];

    let result = translate_anthropic_tools_to_openai(&tools);

    assert_eq!(result.tools[0].function.name, name);
    assert!(result.name_mapping.is_empty());
}

#[test]
fn name_at_65_chars_is_truncated() {
    let name = "a".repeat(65);
    let tools = vec![ToolDefinition {
        name: name.clone(),
        description: None,
        input_schema: InputSchema {
            schema_type: "object".to_string(),
            properties: None,
            required: None,
        },
    }];

    let result = translate_anthropic_tools_to_openai(&tools);

    let truncated_name = &result.tools[0].function.name;
    assert_eq!(truncated_name.chars().count(), 64);
    assert!(result.name_mapping.contains_key(truncated_name));
    assert_eq!(result.name_mapping[truncated_name], name);
}

#[test]
fn truncated_name_is_deterministic() {
    let name = "very_long_tool_name_that_exceeds_the_sixty_four_character_limit_by_quite_a_bit";
    let tools = vec![ToolDefinition {
        name: name.to_string(),
        description: None,
        input_schema: InputSchema {
            schema_type: "object".to_string(),
            properties: None,
            required: None,
        },
    }];

    let result1 = translate_anthropic_tools_to_openai(&tools);
    let result2 = translate_anthropic_tools_to_openai(&tools);

    assert_eq!(
        result1.tools[0].function.name,
        result2.tools[0].function.name
    );
}

#[test]
fn truncated_name_has_hash_suffix() {
    let name =
        "a_very_long_tool_name_that_definitely_exceeds_sixty_four_characters_in_total_length";
    let tools = vec![ToolDefinition {
        name: name.to_string(),
        description: None,
        input_schema: InputSchema {
            schema_type: "object".to_string(),
            properties: None,
            required: None,
        },
    }];

    let result = translate_anthropic_tools_to_openai(&tools);
    let truncated = &result.tools[0].function.name;

    // Format is: 55-char prefix + "_" + 8-char hash = 64
    assert_eq!(truncated.chars().count(), 64);
    assert_eq!(
        truncated.chars().nth(55),
        Some('_'),
        "Character at index 55 should be the '_' separator"
    );

    // Hash part (chars 56..64) should be valid hex
    let hash_part: String = truncated.chars().skip(56).collect();
    assert_eq!(hash_part.chars().count(), 8);
    assert!(hash_part.chars().all(|c| c.is_ascii_hexdigit()));
}

#[test]
fn unicode_name_truncation_respects_char_count() {
    // Build a name with multi-byte characters (CJK) that exceeds 64 chars().count().
    // Each CJK char is 3 bytes in UTF-8, so 65 CJK chars = 195 bytes but only 65 chars.
    let name: String = std::iter::repeat('天').take(65).collect();
    assert_eq!(name.chars().count(), 65);
    assert!(
        name.len() > 65,
        "Name should be >65 bytes (multi-byte chars)"
    );

    let tools = vec![ToolDefinition {
        name: name.clone(),
        description: None,
        input_schema: InputSchema {
            schema_type: "object".to_string(),
            properties: None,
            required: None,
        },
    }];

    let result = translate_anthropic_tools_to_openai(&tools);

    let truncated_name = &result.tools[0].function.name;
    // Truncated name must be exactly 64 characters (not bytes).
    assert_eq!(
        truncated_name.chars().count(),
        64,
        "Truncated name should be 64 chars, got {} chars ({} bytes)",
        truncated_name.chars().count(),
        truncated_name.len()
    );
    // Byte length will be larger than 64 due to multi-byte CJK characters.
    assert!(
        truncated_name.len() > 64,
        "Truncated name should be >64 bytes for multi-byte chars"
    );
    // Roundtrip should restore original name.
    assert!(result.name_mapping.contains_key(truncated_name));
    assert_eq!(result.name_mapping[truncated_name], name);
    let restored = restore_tool_name(truncated_name, &result.name_mapping);
    assert_eq!(restored, name);
}

// ---------------------------------------------------------------------------
// Name mapping roundtrip
// ---------------------------------------------------------------------------

#[test]
fn restore_truncated_name() {
    let name = "x".repeat(100);
    let tools = vec![ToolDefinition {
        name: name.clone(),
        description: None,
        input_schema: InputSchema {
            schema_type: "object".to_string(),
            properties: None,
            required: None,
        },
    }];

    let result = translate_anthropic_tools_to_openai(&tools);
    let truncated = &result.tools[0].function.name;

    let restored = restore_tool_name(truncated, &result.name_mapping);
    assert_eq!(restored, name);
}

#[test]
fn restore_non_truncated_name() {
    let mapping = HashMap::new();
    let restored = restore_tool_name("bash", &mapping);
    assert_eq!(restored, "bash");
}

#[test]
fn restore_unknown_name_returns_original() {
    let mapping = HashMap::new();
    let restored = restore_tool_name("unknown_tool", &mapping);
    assert_eq!(restored, "unknown_tool");
}

#[test]
fn roundtrip_with_mixed_names() {
    let tools = vec![
        ToolDefinition {
            name: "short".to_string(),
            description: None,
            input_schema: InputSchema {
                schema_type: "object".to_string(),
                properties: None,
                required: None,
            },
        },
        ToolDefinition {
            name: "a_very_long_tool_name_that_absolutely_exceeds_the_64_character_limit_imposed_by_openai".to_string(),
            description: None,
            input_schema: InputSchema {
                schema_type: "object".to_string(),
                properties: None,
                required: None,
            },
        },
    ];

    let result = translate_anthropic_tools_to_openai(&tools);

    // Short name: no mapping
    let restored_short = restore_tool_name(&result.tools[0].function.name, &result.name_mapping);
    assert_eq!(restored_short, "short");

    // Long name: mapping roundtrips
    let restored_long = restore_tool_name(&result.tools[1].function.name, &result.name_mapping);
    assert_eq!(
        restored_long,
        "a_very_long_tool_name_that_absolutely_exceeds_the_64_character_limit_imposed_by_openai"
    );
}

// ---------------------------------------------------------------------------
// Schema preservation
// ---------------------------------------------------------------------------

#[test]
fn schema_type_preserved() {
    let tools = vec![ToolDefinition {
        name: "test_tool".to_string(),
        description: None,
        input_schema: InputSchema {
            schema_type: "object".to_string(),
            properties: None,
            required: None,
        },
    }];

    let result = translate_anthropic_tools_to_openai(&tools);
    let params = result.tools[0].function.parameters.as_ref().unwrap();
    assert_eq!(params["type"], "object");
}

#[test]
fn schema_properties_preserved() {
    let properties = serde_json::json!({
        "path": {
            "type": "string",
            "description": "File path to read"
        },
        "encoding": {
            "type": "string",
            "enum": ["utf-8", "ascii", "latin-1"]
        }
    });

    let tools = vec![ToolDefinition {
        name: "read_file".to_string(),
        description: Some("Read a file".to_string()),
        input_schema: InputSchema {
            schema_type: "object".to_string(),
            properties: Some(properties.clone()),
            required: Some(vec!["path".to_string()]),
        },
    }];

    let result = translate_anthropic_tools_to_openai(&tools);
    let params = result.tools[0].function.parameters.as_ref().unwrap();

    assert_eq!(params["properties"], properties);
    assert_eq!(params["required"][0], "path");
}

#[test]
fn schema_without_properties_or_required() {
    let tools = vec![ToolDefinition {
        name: "noop".to_string(),
        description: None,
        input_schema: InputSchema {
            schema_type: "object".to_string(),
            properties: None,
            required: None,
        },
    }];

    let result = translate_anthropic_tools_to_openai(&tools);
    let params = result.tools[0].function.parameters.as_ref().unwrap();

    assert_eq!(params["type"], "object");
    assert!(params.get("properties").is_none());
    assert!(params.get("required").is_none());
}

#[test]
fn schema_with_nested_objects_preserved() {
    let properties = serde_json::json!({
        "config": {
            "type": "object",
            "properties": {
                "timeout": { "type": "integer" },
                "retries": { "type": "integer" }
            }
        },
        "items": {
            "type": "array",
            "items": { "type": "string" }
        }
    });

    let tools = vec![ToolDefinition {
        name: "complex_tool".to_string(),
        description: Some("A tool with complex schema".to_string()),
        input_schema: InputSchema {
            schema_type: "object".to_string(),
            properties: Some(properties.clone()),
            required: Some(vec!["config".to_string()]),
        },
    }];

    let result = translate_anthropic_tools_to_openai(&tools);
    let params = result.tools[0].function.parameters.as_ref().unwrap();

    assert_eq!(params["properties"]["config"]["type"], "object");
    assert_eq!(
        params["properties"]["config"]["properties"]["timeout"]["type"],
        "integer"
    );
    assert_eq!(params["properties"]["items"]["type"], "array");
    assert_eq!(params["properties"]["items"]["items"]["type"], "string");
}

#[test]
fn translated_tool_serializes_to_valid_json() {
    let tools = vec![ToolDefinition {
        name: "get_weather".to_string(),
        description: Some("Get the weather for a location".to_string()),
        input_schema: InputSchema {
            schema_type: "object".to_string(),
            properties: Some(serde_json::json!({
                "location": {
                    "type": "string",
                    "description": "The city name"
                }
            })),
            required: Some(vec!["location".to_string()]),
        },
    }];

    let result = translate_anthropic_tools_to_openai(&tools);
    let json = serde_json::to_value(&result.tools[0]).unwrap();

    assert_eq!(json["type"], "function");
    assert_eq!(json["function"]["name"], "get_weather");
    assert_eq!(
        json["function"]["description"],
        "Get the weather for a location"
    );
    assert_eq!(json["function"]["parameters"]["type"], "object");
    assert_eq!(
        json["function"]["parameters"]["properties"]["location"]["type"],
        "string"
    );
    assert_eq!(json["function"]["parameters"]["required"][0], "location");
}

#[test]
fn translated_tool_without_optional_fields_omits_them_in_json() {
    let tools = vec![ToolDefinition {
        name: "noop".to_string(),
        description: None,
        input_schema: InputSchema {
            schema_type: "object".to_string(),
            properties: None,
            required: None,
        },
    }];

    let result = translate_anthropic_tools_to_openai(&tools);
    let json = serde_json::to_value(&result.tools[0]).unwrap();

    assert_eq!(json["type"], "function");
    assert_eq!(json["function"]["name"], "noop");
    assert!(json["function"].get("description").is_none());
    // parameters is always Some (contains at least {"type": "object"})
    assert!(json["function"].get("parameters").is_some());
}
