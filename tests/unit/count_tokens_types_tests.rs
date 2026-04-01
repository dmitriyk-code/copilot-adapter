use copilot_adapter::anthropic::types::*;

// ---------------------------------------------------------------------------
// E2-T3: Deserialize minimal CountTokensRequest (model + messages only)
// ---------------------------------------------------------------------------

#[test]
fn count_tokens_request_minimal_deserializes() {
    let json = serde_json::json!({
        "model": "claude-sonnet-4-20250514",
        "messages": [{"role": "user", "content": "Hello"}]
    });
    let req: CountTokensRequest = serde_json::from_value(json).unwrap();
    assert_eq!(req.model, "claude-sonnet-4-20250514");
    assert_eq!(req.messages.len(), 1);
    assert_eq!(req.messages[0].role, "user");
    assert!(req.system.is_none());
    assert!(req.tools.is_none());
}

#[test]
fn count_tokens_request_with_multiple_messages() {
    let json = serde_json::json!({
        "model": "claude-sonnet-4-20250514",
        "messages": [
            {"role": "user", "content": "Hello"},
            {"role": "assistant", "content": "Hi!"},
            {"role": "user", "content": "How are you?"}
        ]
    });
    let req: CountTokensRequest = serde_json::from_value(json).unwrap();
    assert_eq!(req.messages.len(), 3);
    assert_eq!(req.messages[0].role, "user");
    assert_eq!(req.messages[1].role, "assistant");
    assert_eq!(req.messages[2].role, "user");
}

// ---------------------------------------------------------------------------
// E2-T4: Deserialize CountTokensRequest with optional fields
// ---------------------------------------------------------------------------

#[test]
fn count_tokens_request_with_system_string_deserializes() {
    let json = serde_json::json!({
        "model": "claude-sonnet-4-20250514",
        "messages": [{"role": "user", "content": "Hello"}],
        "system": "You are a helpful assistant."
    });
    let req: CountTokensRequest = serde_json::from_value(json).unwrap();
    assert_eq!(
        req.system.as_ref().map(|s| s.to_text()),
        Some("You are a helpful assistant.".to_string())
    );
}

#[test]
fn count_tokens_request_with_system_blocks_deserializes() {
    let json = serde_json::json!({
        "model": "claude-sonnet-4-20250514",
        "messages": [{"role": "user", "content": "Hello"}],
        "system": [{"type": "text", "text": "You are a helpful assistant."}]
    });
    let req: CountTokensRequest = serde_json::from_value(json).unwrap();
    assert_eq!(
        req.system.as_ref().map(|s| s.to_text()),
        Some("You are a helpful assistant.".to_string())
    );
}

#[test]
fn count_tokens_request_with_tools_deserializes() {
    let json = serde_json::json!({
        "model": "claude-sonnet-4-20250514",
        "messages": [{"role": "user", "content": "Search for foo"}],
        "tools": [{
            "name": "grep",
            "description": "Search files for a pattern",
            "input_schema": {
                "type": "object",
                "properties": {
                    "pattern": {"type": "string"}
                },
                "required": ["pattern"]
            }
        }]
    });
    let req: CountTokensRequest = serde_json::from_value(json).unwrap();
    let tools = req.tools.unwrap();
    assert_eq!(tools.len(), 1);
    assert_eq!(tools[0].name, "grep");
    assert_eq!(tools[0].description, Some("Search files for a pattern".to_string()));
}

#[test]
fn count_tokens_request_full_deserializes() {
    let json = serde_json::json!({
        "model": "claude-sonnet-4-20250514",
        "messages": [
            {"role": "user", "content": "Search for foo"},
            {"role": "assistant", "content": [{"type": "tool_use", "id": "tu_1", "name": "grep", "input": {"pattern": "foo"}}]},
            {"role": "user", "content": [{"type": "tool_result", "tool_use_id": "tu_1", "content": "found foo in bar.txt"}]}
        ],
        "system": "You are a coding assistant.",
        "tools": [
            {
                "name": "grep",
                "description": "Search files",
                "input_schema": {"type": "object", "properties": {"pattern": {"type": "string"}}}
            },
            {
                "name": "read_file",
                "description": "Read a file",
                "input_schema": {"type": "object", "properties": {"path": {"type": "string"}}, "required": ["path"]}
            }
        ]
    });
    let req: CountTokensRequest = serde_json::from_value(json).unwrap();
    assert_eq!(req.model, "claude-sonnet-4-20250514");
    assert_eq!(req.messages.len(), 3);
    assert!(req.system.is_some());
    assert_eq!(req.tools.as_ref().unwrap().len(), 2);
}

// ---------------------------------------------------------------------------
// CountTokensResponse serialization
// ---------------------------------------------------------------------------

#[test]
fn count_tokens_response_serializes_to_json() {
    let resp = CountTokensResponse { input_tokens: 42 };
    let json = serde_json::to_value(&resp).unwrap();
    assert_eq!(json, serde_json::json!({"input_tokens": 42}));
}

#[test]
fn count_tokens_response_serializes_zero() {
    let resp = CountTokensResponse { input_tokens: 0 };
    let json = serde_json::to_value(&resp).unwrap();
    assert_eq!(json, serde_json::json!({"input_tokens": 0}));
}

#[test]
fn count_tokens_response_serializes_large_value() {
    let resp = CountTokensResponse { input_tokens: 200_000 };
    let json = serde_json::to_value(&resp).unwrap();
    assert_eq!(json, serde_json::json!({"input_tokens": 200000}));
}

// ---------------------------------------------------------------------------
// Edge cases
// ---------------------------------------------------------------------------

#[test]
fn count_tokens_request_with_content_blocks_deserializes() {
    let json = serde_json::json!({
        "model": "claude-sonnet-4-20250514",
        "messages": [{
            "role": "user",
            "content": [
                {"type": "text", "text": "Hello "},
                {"type": "text", "text": "world!"}
            ]
        }]
    });
    let req: CountTokensRequest = serde_json::from_value(json).unwrap();
    assert_eq!(req.messages.len(), 1);
}

#[test]
fn count_tokens_request_with_tool_without_input_schema_uses_default() {
    let json = serde_json::json!({
        "model": "claude-sonnet-4-20250514",
        "messages": [{"role": "user", "content": "Hello"}],
        "tools": [{"name": "simple_tool"}]
    });
    let req: CountTokensRequest = serde_json::from_value(json).unwrap();
    let tools = req.tools.unwrap();
    assert_eq!(tools[0].input_schema.schema_type, "object");
}
