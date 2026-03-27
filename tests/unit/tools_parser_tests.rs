use copilot_adapter::tools::parser::{parse_tool_calls, strip_tool_calls};

// ---------------------------------------------------------------------------
// E3-T9: Parse single tool call from fenced code block
// ---------------------------------------------------------------------------

#[test]
fn parse_single_fenced_tool_call() {
    let content = r#"Sure, let me check the weather for you.

```json
{"function_call": {"name": "get_weather", "arguments": {"location": "London"}}}
```

I'll get that information now."#;

    let tool_calls = parse_tool_calls(content);

    assert_eq!(tool_calls.len(), 1);
    let tc = &tool_calls[0];
    assert!(tc.id.as_ref().unwrap().starts_with("call_"));
    assert_eq!(tc.call_type, Some("function".to_string()));
    assert_eq!(tc.function.name, Some("get_weather".to_string()));

    // Arguments preserved as raw JSON string
    let args: serde_json::Value =
        serde_json::from_str(tc.function.arguments.as_ref().unwrap()).unwrap();
    assert_eq!(args["location"], "London");
}

#[test]
fn parse_single_inline_tool_call() {
    let content =
        r#"Here is the result: {"function_call": {"name": "bash", "arguments": {"command": "ls -la"}}}"#;

    let tool_calls = parse_tool_calls(content);

    assert_eq!(tool_calls.len(), 1);
    assert_eq!(tool_calls[0].function.name, Some("bash".to_string()));

    let args: serde_json::Value =
        serde_json::from_str(tool_calls[0].function.arguments.as_ref().unwrap()).unwrap();
    assert_eq!(args["command"], "ls -la");
}

// ---------------------------------------------------------------------------
// E3-T10: Parse multiple tool calls
// ---------------------------------------------------------------------------

#[test]
fn parse_multiple_fenced_tool_calls() {
    let content = r#"I'll read both files for you.

```json
{"function_call": {"name": "read_file", "arguments": {"path": "/tmp/a.txt"}}}
```

And then the second one:

```json
{"function_call": {"name": "read_file", "arguments": {"path": "/tmp/b.txt"}}}
```

Done."#;

    let tool_calls = parse_tool_calls(content);

    assert_eq!(tool_calls.len(), 2);
    assert_eq!(tool_calls[0].function.name, Some("read_file".to_string()));
    assert_eq!(tool_calls[1].function.name, Some("read_file".to_string()));

    let args0: serde_json::Value =
        serde_json::from_str(tool_calls[0].function.arguments.as_ref().unwrap()).unwrap();
    let args1: serde_json::Value =
        serde_json::from_str(tool_calls[1].function.arguments.as_ref().unwrap()).unwrap();
    assert_eq!(args0["path"], "/tmp/a.txt");
    assert_eq!(args1["path"], "/tmp/b.txt");

    // Each has a unique ID
    assert_ne!(tool_calls[0].id, tool_calls[1].id);
}

#[test]
fn parse_multiple_inline_tool_calls() {
    let content = r#"{"function_call": {"name": "tool_a", "arguments": {}}} and {"function_call": {"name": "tool_b", "arguments": {}}}"#;

    let tool_calls = parse_tool_calls(content);

    assert_eq!(tool_calls.len(), 2);
    assert_eq!(tool_calls[0].function.name, Some("tool_a".to_string()));
    assert_eq!(tool_calls[1].function.name, Some("tool_b".to_string()));
}

#[test]
fn parse_mixed_fenced_and_inline_tool_calls() {
    let content = r#"First tool:

```json
{"function_call": {"name": "fenced_tool", "arguments": {"key": "value"}}}
```

Second tool: {"function_call": {"name": "inline_tool", "arguments": {"x": 42}}}"#;

    let tool_calls = parse_tool_calls(content);

    assert_eq!(tool_calls.len(), 2);
    // Should be in document order
    assert_eq!(
        tool_calls[0].function.name,
        Some("fenced_tool".to_string())
    );
    assert_eq!(
        tool_calls[1].function.name,
        Some("inline_tool".to_string())
    );
}

// ---------------------------------------------------------------------------
// E3-T11: Parse tool call with complex nested arguments
// ---------------------------------------------------------------------------

#[test]
fn parse_tool_call_with_nested_arguments() {
    let content = r#"```json
{"function_call": {"name": "create_file", "arguments": {"path": "/tmp/config.json", "content": "{\"key\": \"value\", \"nested\": {\"a\": [1, 2, 3]}}"}}}
```"#;

    let tool_calls = parse_tool_calls(content);

    assert_eq!(tool_calls.len(), 1);
    assert_eq!(
        tool_calls[0].function.name,
        Some("create_file".to_string())
    );

    let args: serde_json::Value =
        serde_json::from_str(tool_calls[0].function.arguments.as_ref().unwrap()).unwrap();
    assert_eq!(args["path"], "/tmp/config.json");
    // The content field itself contains JSON as a string
    assert!(args["content"].is_string());
}

#[test]
fn parse_tool_call_with_array_arguments() {
    let content = r#"```json
{"function_call": {"name": "multi_search", "arguments": {"queries": ["rust", "tokio", "async"], "limit": 10, "options": {"case_sensitive": false}}}}
```"#;

    let tool_calls = parse_tool_calls(content);

    assert_eq!(tool_calls.len(), 1);
    let args: serde_json::Value =
        serde_json::from_str(tool_calls[0].function.arguments.as_ref().unwrap()).unwrap();
    assert_eq!(args["queries"][0], "rust");
    assert_eq!(args["queries"][1], "tokio");
    assert_eq!(args["queries"][2], "async");
    assert_eq!(args["limit"], 10);
    assert_eq!(args["options"]["case_sensitive"], false);
}

#[test]
fn parse_tool_call_with_deeply_nested_object() {
    let content = r#"```json
{"function_call": {"name": "deploy", "arguments": {"config": {"env": "production", "replicas": 3, "resources": {"cpu": "500m", "memory": "1Gi"}}}}}
```"#;

    let tool_calls = parse_tool_calls(content);

    assert_eq!(tool_calls.len(), 1);
    let args: serde_json::Value =
        serde_json::from_str(tool_calls[0].function.arguments.as_ref().unwrap()).unwrap();
    assert_eq!(args["config"]["env"], "production");
    assert_eq!(args["config"]["replicas"], 3);
    assert_eq!(args["config"]["resources"]["cpu"], "500m");
    assert_eq!(args["config"]["resources"]["memory"], "1Gi");
}

// ---------------------------------------------------------------------------
// E3-T12: No tool calls found returns empty vec
// ---------------------------------------------------------------------------

#[test]
fn no_tool_calls_returns_empty() {
    let content = "Just a regular message with no tool calls at all.";
    let tool_calls = parse_tool_calls(content);
    assert!(tool_calls.is_empty());
}

#[test]
fn empty_string_returns_empty() {
    let tool_calls = parse_tool_calls("");
    assert!(tool_calls.is_empty());
}

#[test]
fn json_without_function_call_key_returns_empty() {
    let content = r#"```json
{"name": "bash", "arguments": {"command": "ls"}}
```"#;

    let tool_calls = parse_tool_calls(content);
    assert!(tool_calls.is_empty());
}

#[test]
fn regular_code_block_not_matched() {
    let content = r#"Here's some code:

```json
{"key": "value", "items": [1, 2, 3]}
```

That's a sample JSON document."#;

    let tool_calls = parse_tool_calls(content);
    assert!(tool_calls.is_empty());
}

// ---------------------------------------------------------------------------
// E3-T13: Malformed JSON gracefully skipped
// ---------------------------------------------------------------------------

#[test]
fn malformed_json_in_fenced_block_skipped() {
    let content = r#"```json
{not valid json at all
```"#;

    let tool_calls = parse_tool_calls(content);
    assert!(tool_calls.is_empty());
}

#[test]
fn partially_valid_json_skips_bad_parses_valid() {
    let content = r#"```json
{this is broken}
```

```json
{"function_call": {"name": "valid_tool", "arguments": {"key": "value"}}}
```"#;

    let tool_calls = parse_tool_calls(content);

    assert_eq!(tool_calls.len(), 1);
    assert_eq!(
        tool_calls[0].function.name,
        Some("valid_tool".to_string())
    );
}

#[test]
fn function_call_missing_name_skipped() {
    let content = r#"```json
{"function_call": {"arguments": {"key": "value"}}}
```"#;

    let tool_calls = parse_tool_calls(content);
    assert!(tool_calls.is_empty());
}

#[test]
fn function_call_with_non_string_name_skipped() {
    let content = r#"```json
{"function_call": {"name": 123, "arguments": {}}}
```"#;

    let tool_calls = parse_tool_calls(content);
    assert!(tool_calls.is_empty());
}

// ---------------------------------------------------------------------------
// E3-T14: strip_tool_calls removes tool call text
// ---------------------------------------------------------------------------

#[test]
fn strip_fenced_tool_call() {
    let content = r#"Here is some text.

```json
{"function_call": {"name": "bash", "arguments": {"command": "ls"}}}
```

And more text after."#;

    let stripped = strip_tool_calls(content);

    assert!(!stripped.contains("function_call"));
    assert!(!stripped.contains("```json"));
    assert!(stripped.contains("Here is some text."));
    assert!(stripped.contains("And more text after."));
}

#[test]
fn strip_multiple_tool_calls() {
    let content = r#"First paragraph.

```json
{"function_call": {"name": "tool_a", "arguments": {}}}
```

Middle text.

```json
{"function_call": {"name": "tool_b", "arguments": {}}}
```

Last paragraph."#;

    let stripped = strip_tool_calls(content);

    assert!(!stripped.contains("function_call"));
    assert!(!stripped.contains("tool_a"));
    assert!(!stripped.contains("tool_b"));
    assert!(stripped.contains("First paragraph."));
    assert!(stripped.contains("Middle text."));
    assert!(stripped.contains("Last paragraph."));
}

#[test]
fn strip_inline_tool_call() {
    let content =
        r#"The result is {"function_call": {"name": "bash", "arguments": {"command": "pwd"}}} done."#;

    let stripped = strip_tool_calls(content);

    assert!(!stripped.contains("function_call"));
    assert!(stripped.contains("The result is"));
    assert!(stripped.contains("done."));
}

#[test]
fn strip_no_tool_calls_returns_original() {
    let content = "Just regular text with no tool calls.";
    let stripped = strip_tool_calls(content);
    assert_eq!(stripped, content);
}

#[test]
fn strip_regular_json_block_is_preserved() {
    let content = r#"See the error format:

```json
{"error": "not found", "code": 404}
```

That block should remain."#;

    let stripped = strip_tool_calls(content);

    // The regular JSON block must NOT be removed
    assert!(stripped.contains(r#"{"error": "not found", "code": 404}"#));
    assert!(stripped.contains("```json"));
    assert!(stripped.contains("See the error format:"));
    assert!(stripped.contains("That block should remain."));
}

#[test]
fn strip_removes_tool_call_but_preserves_regular_json_block() {
    let content = r#"Here is an error example:

```json
{"error": "not found", "code": 404}
```

Now calling a tool:

```json
{"function_call": {"name": "bash", "arguments": {"command": "ls"}}}
```

Done."#;

    let stripped = strip_tool_calls(content);

    // Regular JSON block preserved
    assert!(stripped.contains(r#"{"error": "not found", "code": 404}"#));
    // Tool call block removed
    assert!(!stripped.contains("function_call"));
    assert!(stripped.contains("Here is an error example:"));
    assert!(stripped.contains("Done."));
}

#[test]
fn strip_inline_skips_invalid_and_removes_valid() {
    // First inline has function_call key but missing name → invalid, should be skipped
    // Second inline is valid → should be removed
    let content = r#"Before {"function_call": {"invalid": true}} middle {"function_call": {"name": "valid", "arguments": {}}} after"#;

    let stripped = strip_tool_calls(content);

    // The invalid one should still be in the output
    assert!(stripped.contains(r#"{"function_call": {"invalid": true}}"#));
    // The valid one should be removed
    assert!(!stripped.contains(r#""name": "valid""#));
    assert!(stripped.contains("Before"));
    assert!(stripped.contains("after"));
}

#[test]
fn strip_collapses_excessive_blank_lines() {
    let content = r#"Before.



```json
{"function_call": {"name": "test", "arguments": {}}}
```



After."#;

    let stripped = strip_tool_calls(content);

    // Should not have more than one blank line between paragraphs
    assert!(!stripped.contains("\n\n\n"));
    assert!(stripped.contains("Before."));
    assert!(stripped.contains("After."));
}

// ---------------------------------------------------------------------------
// Additional edge cases
// ---------------------------------------------------------------------------

#[test]
fn tool_call_ids_are_unique_across_calls() {
    let content = r#"```json
{"function_call": {"name": "a", "arguments": {}}}
```

```json
{"function_call": {"name": "b", "arguments": {}}}
```

```json
{"function_call": {"name": "c", "arguments": {}}}
```"#;

    let tool_calls = parse_tool_calls(content);

    assert_eq!(tool_calls.len(), 3);
    let ids: Vec<&str> = tool_calls
        .iter()
        .map(|tc| tc.id.as_ref().unwrap().as_str())
        .collect();

    // All unique
    let mut unique_ids = ids.clone();
    unique_ids.sort();
    unique_ids.dedup();
    assert_eq!(ids.len(), unique_ids.len());

    // All start with call_
    for id in &ids {
        assert!(id.starts_with("call_"));
    }
}

#[test]
fn arguments_without_value_stored_as_none() {
    // A function_call that has name but no arguments key at all
    let content = r#"```json
{"function_call": {"name": "noop"}}
```"#;

    let tool_calls = parse_tool_calls(content);

    assert_eq!(tool_calls.len(), 1);
    assert_eq!(tool_calls[0].function.name, Some("noop".to_string()));
    assert!(tool_calls[0].function.arguments.is_none());
}

#[test]
fn arguments_preserved_as_raw_json_string() {
    let content = r#"```json
{"function_call": {"name": "test", "arguments": {"count": 42, "active": true, "name": "hello"}}}
```"#;

    let tool_calls = parse_tool_calls(content);

    assert_eq!(tool_calls.len(), 1);
    let args_str = tool_calls[0].function.arguments.as_ref().unwrap();
    // Should be valid JSON when parsed
    let parsed: serde_json::Value = serde_json::from_str(args_str).unwrap();
    assert_eq!(parsed["count"], 42);
    assert_eq!(parsed["active"], true);
    assert_eq!(parsed["name"], "hello");
}
