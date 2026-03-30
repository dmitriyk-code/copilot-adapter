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

// ---------------------------------------------------------------------------
// XML parsing tests — Epic 2
// ---------------------------------------------------------------------------

#[test]
fn parse_single_xml_tool_call() {
    let content = r#"<function_calls>
<invoke name="Bash">
<parameter name="command">ls -la</parameter>
</invoke>
</function_calls>"#;

    let tool_calls = parse_tool_calls(content);

    assert_eq!(tool_calls.len(), 1);
    let tc = &tool_calls[0];
    assert!(tc.id.as_ref().unwrap().starts_with("call_"));
    assert_eq!(tc.call_type, Some("function".to_string()));
    assert_eq!(tc.function.name, Some("Bash".to_string()));

    let args: serde_json::Value =
        serde_json::from_str(tc.function.arguments.as_ref().unwrap()).unwrap();
    assert_eq!(args["command"], "ls -la");
}

#[test]
fn parse_xml_with_multiple_parameters() {
    let content = r#"<function_calls>
<invoke name="Bash">
<parameter name="command">git mv MISSING-FEATURES.md docs/</parameter>
<parameter name="description">Move file</parameter>
</invoke>
</function_calls>"#;

    let tool_calls = parse_tool_calls(content);

    assert_eq!(tool_calls.len(), 1);
    assert_eq!(tool_calls[0].function.name, Some("Bash".to_string()));

    let args: serde_json::Value =
        serde_json::from_str(tool_calls[0].function.arguments.as_ref().unwrap()).unwrap();
    assert_eq!(args["command"], "git mv MISSING-FEATURES.md docs/");
    assert_eq!(args["description"], "Move file");
}

#[test]
fn parse_multiple_xml_invokes() {
    let content = r#"<function_calls>
<invoke name="Bash">
<parameter name="command">ls</parameter>
</invoke>
<invoke name="Grep">
<parameter name="pattern">test</parameter>
</invoke>
</function_calls>"#;

    let tool_calls = parse_tool_calls(content);

    assert_eq!(tool_calls.len(), 2);
    assert_eq!(tool_calls[0].function.name, Some("Bash".to_string()));
    assert_eq!(tool_calls[1].function.name, Some("Grep".to_string()));

    // Each has a unique ID
    assert_ne!(tool_calls[0].id, tool_calls[1].id);
}

#[test]
fn parse_xml_with_surrounding_text() {
    let content = r#"Let me check that for you.

<function_calls>
<invoke name="Bash">
<parameter name="command">ls -la</parameter>
</invoke>
</function_calls>

I'll run that now."#;

    let tool_calls = parse_tool_calls(content);

    assert_eq!(tool_calls.len(), 1);
    assert_eq!(tool_calls[0].function.name, Some("Bash".to_string()));
}

#[test]
fn parse_xml_no_parameters() {
    let content = r#"<function_calls><invoke name="NoOp"></invoke></function_calls>"#;

    let tool_calls = parse_tool_calls(content);

    assert_eq!(tool_calls.len(), 1);
    assert_eq!(tool_calls[0].function.name, Some("NoOp".to_string()));

    let args: serde_json::Value =
        serde_json::from_str(tool_calls[0].function.arguments.as_ref().unwrap()).unwrap();
    assert!(args.as_object().unwrap().is_empty());
}

#[test]
fn parse_xml_malformed_missing_name_attribute() {
    let content =
        r#"<function_calls><invoke><parameter name="x">y</parameter></invoke></function_calls>"#;

    let tool_calls = parse_tool_calls(content);
    assert!(tool_calls.is_empty());
}

#[test]
fn parse_xml_empty_function_calls() {
    let content = "<function_calls></function_calls>";

    let tool_calls = parse_tool_calls(content);
    assert!(tool_calls.is_empty());
}

#[test]
fn parse_xml_empty_name_returns_empty() {
    let content = r#"<function_calls><invoke name=""><parameter name="x">y</parameter></invoke></function_calls>"#;

    let tool_calls = parse_tool_calls(content);
    assert!(tool_calls.is_empty());
}

#[test]
fn parse_mixed_json_and_xml_prefers_json() {
    let content = r#"
```json
{"function_call": {"name": "JsonTool", "arguments": {}}}
```

<function_calls>
<invoke name="XmlTool"><parameter name="x">y</parameter></invoke>
</function_calls>
"#;

    let tool_calls = parse_tool_calls(content);

    // JSON takes priority over XML
    assert_eq!(tool_calls.len(), 1);
    assert_eq!(tool_calls[0].function.name, Some("JsonTool".to_string()));
}

#[test]
fn parse_xml_with_whitespace_in_tags() {
    let content = r#"<function_calls>
  <invoke name="Bash">
    <parameter name="command">echo hello</parameter>
  </invoke>
</function_calls>"#;

    let tool_calls = parse_tool_calls(content);

    assert_eq!(tool_calls.len(), 1);
    assert_eq!(tool_calls[0].function.name, Some("Bash".to_string()));

    let args: serde_json::Value =
        serde_json::from_str(tool_calls[0].function.arguments.as_ref().unwrap()).unwrap();
    assert_eq!(args["command"], "echo hello");
}

#[test]
fn parse_xml_multiple_function_calls_blocks() {
    let content = r#"First block:
<function_calls>
<invoke name="Tool1"><parameter name="a">1</parameter></invoke>
</function_calls>

Second block:
<function_calls>
<invoke name="Tool2"><parameter name="b">2</parameter></invoke>
</function_calls>"#;

    let tool_calls = parse_tool_calls(content);

    assert_eq!(tool_calls.len(), 2);
    assert_eq!(tool_calls[0].function.name, Some("Tool1".to_string()));
    assert_eq!(tool_calls[1].function.name, Some("Tool2".to_string()));
}

#[test]
fn parse_xml_no_function_calls_wrapper_returns_empty() {
    // Bare invoke without function_calls wrapper should not match
    let content = r#"<invoke name="Bash"><parameter name="command">ls</parameter></invoke>"#;

    let tool_calls = parse_tool_calls(content);
    assert!(tool_calls.is_empty());
}

#[test]
fn xml_tool_call_ids_are_unique() {
    let content = r#"<function_calls>
<invoke name="A"><parameter name="x">1</parameter></invoke>
<invoke name="B"><parameter name="y">2</parameter></invoke>
<invoke name="C"><parameter name="z">3</parameter></invoke>
</function_calls>"#;

    let tool_calls = parse_tool_calls(content);

    assert_eq!(tool_calls.len(), 3);
    let ids: Vec<&str> = tool_calls
        .iter()
        .map(|tc| tc.id.as_ref().unwrap().as_str())
        .collect();

    let mut unique_ids = ids.clone();
    unique_ids.sort();
    unique_ids.dedup();
    assert_eq!(ids.len(), unique_ids.len());
}

#[test]
fn parse_xml_parameters_are_all_strings() {
    // XML parameters are always extracted as strings
    let content = r#"<function_calls>
<invoke name="Test">
<parameter name="count">42</parameter>
<parameter name="active">true</parameter>
<parameter name="name">hello</parameter>
</invoke>
</function_calls>"#;

    let tool_calls = parse_tool_calls(content);
    assert_eq!(tool_calls.len(), 1);

    let args: serde_json::Value =
        serde_json::from_str(tool_calls[0].function.arguments.as_ref().unwrap()).unwrap();
    // All values are strings in XML parsing
    assert_eq!(args["count"], "42");
    assert_eq!(args["active"], "true");
    assert_eq!(args["name"], "hello");
}

#[test]
fn no_content_returns_empty_for_both_formats() {
    assert!(parse_tool_calls("").is_empty());
    assert!(parse_tool_calls("just plain text").is_empty());
    assert!(parse_tool_calls("<not_function_calls></not_function_calls>").is_empty());
}

// ---------------------------------------------------------------------------
// XML stripping tests
// ---------------------------------------------------------------------------

#[test]
fn strip_xml_tool_call() {
    let content = r#"Let me check that.

<function_calls>
<invoke name="Bash">
<parameter name="command">ls -la</parameter>
</invoke>
</function_calls>

Done."#;

    let stripped = strip_tool_calls(content);
    assert_eq!(stripped, "Let me check that.\n\nDone.");
    assert!(!stripped.contains("<function_calls>"));
    assert!(!stripped.contains("<invoke"));
}

#[test]
fn strip_xml_multiple_blocks() {
    let content = r#"First:

<function_calls>
<invoke name="Tool1"><parameter name="a">1</parameter></invoke>
</function_calls>

Second:

<function_calls>
<invoke name="Tool2"><parameter name="b">2</parameter></invoke>
</function_calls>

End."#;

    let stripped = strip_tool_calls(content);
    assert!(!stripped.contains("<function_calls>"));
    assert!(stripped.contains("First:"));
    assert!(stripped.contains("Second:"));
    assert!(stripped.contains("End."));
}

#[test]
fn strip_xml_preserves_surrounding_text() {
    let content = r#"Before text.

<function_calls>
<invoke name="Bash"><parameter name="command">echo hi</parameter></invoke>
</function_calls>

After text."#;

    let stripped = strip_tool_calls(content);
    assert!(stripped.starts_with("Before text."));
    assert!(stripped.ends_with("After text."));
    assert!(!stripped.contains("<function_calls>"));
}

#[test]
fn strip_xml_invalid_block_preserved() {
    // A <function_calls> block with no valid invokes should be preserved
    let content = "Some text.\n\n<function_calls></function_calls>\n\nMore text.";

    let stripped = strip_tool_calls(content);
    assert!(stripped.contains("<function_calls>"));
}

#[test]
fn strip_xml_empty_name_block_preserved() {
    // invoke with empty name is invalid — block should be preserved
    let content = r#"Text.

<function_calls>
<invoke name="">
<parameter name="x">y</parameter>
</invoke>
</function_calls>

More."#;

    let stripped = strip_tool_calls(content);
    assert!(stripped.contains("<function_calls>"));
}

// ---------------------------------------------------------------------------
// XML parameter trimming tests
// ---------------------------------------------------------------------------

#[test]
fn parse_xml_trims_parameter_whitespace() {
    let content = r#"<function_calls>
<invoke name="Bash">
<parameter name="command">
  echo hello world
</parameter>
</invoke>
</function_calls>"#;

    let tool_calls = parse_tool_calls(content);
    assert_eq!(tool_calls.len(), 1);

    let args: serde_json::Value =
        serde_json::from_str(tool_calls[0].function.arguments.as_ref().unwrap()).unwrap();
    assert_eq!(args["command"], "echo hello world");
}

#[test]
fn parse_xml_trims_multiline_parameter_value() {
    let content = r#"<function_calls>
<invoke name="WriteFile">
<parameter name="path">  /tmp/test.txt  </parameter>
<parameter name="content">   some content   </parameter>
</invoke>
</function_calls>"#;

    let tool_calls = parse_tool_calls(content);
    assert_eq!(tool_calls.len(), 1);

    let args: serde_json::Value =
        serde_json::from_str(tool_calls[0].function.arguments.as_ref().unwrap()).unwrap();
    assert_eq!(args["path"], "/tmp/test.txt");
    assert_eq!(args["content"], "some content");
}

// ---------------------------------------------------------------------------
// XML empty name guard test (exercises the guard, not just the regex)
// ---------------------------------------------------------------------------

#[test]
fn parse_xml_empty_name_hits_guard() {
    // With [^"]* regex, empty name is captured and rejected by the guard
    let content = r#"<function_calls><invoke name=""><parameter name="x">y</parameter></invoke></function_calls>"#;

    let tool_calls = parse_tool_calls(content);
    assert!(tool_calls.is_empty());
}
