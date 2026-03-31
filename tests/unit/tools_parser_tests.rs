use copilot_adapter::tools::parser::{parse_tool_calls, strip_tool_calls};

// ===========================================================================
// Tag-based XML parsing tests
// ===========================================================================

#[test]
fn parse_wrapped_function_calls_tag_based() {
    let content = r#"
Here's my analysis:

<function_calls>
<invoke>
<tool_name>Read</tool_name>
<parameters>
<file_path>/src/main.rs</file_path>
</parameters>
</invoke>
</function_calls>
"#;
    let calls = parse_tool_calls(content, None, false);
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].function.name, Some("Read".to_string()));

    let args: serde_json::Value =
        serde_json::from_str(calls[0].function.arguments.as_ref().unwrap()).unwrap();
    assert_eq!(args["file_path"], "/src/main.rs");
}

#[test]
fn parse_standalone_invoke_tag_based() {
    let content = r#"
<invoke>
<tool_name>Edit</tool_name>
<parameters>
<file_path>/src/lib.rs</file_path>
<old_string>old</old_string>
<new_string>new</new_string>
</parameters>
</invoke>
"#;
    let calls = parse_tool_calls(content, None, false);
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].function.name, Some("Edit".to_string()));

    let args: serde_json::Value =
        serde_json::from_str(calls[0].function.arguments.as_ref().unwrap()).unwrap();
    assert_eq!(args["file_path"], "/src/lib.rs");
    assert_eq!(args["old_string"], "old");
    assert_eq!(args["new_string"], "new");
}

#[test]
fn parse_multiple_invokes_tag_based() {
    let content = r#"
<function_calls>
<invoke>
<tool_name>Read</tool_name>
<parameters><file_path>/a.rs</file_path></parameters>
</invoke>
<invoke>
<tool_name>Read</tool_name>
<parameters><file_path>/b.rs</file_path></parameters>
</invoke>
</function_calls>
"#;
    let calls = parse_tool_calls(content, None, false);
    assert_eq!(calls.len(), 2);

    let args0: serde_json::Value =
        serde_json::from_str(calls[0].function.arguments.as_ref().unwrap()).unwrap();
    let args1: serde_json::Value =
        serde_json::from_str(calls[1].function.arguments.as_ref().unwrap()).unwrap();
    assert_eq!(args0["file_path"], "/a.rs");
    assert_eq!(args1["file_path"], "/b.rs");

    // Unique IDs
    assert_ne!(calls[0].id, calls[1].id);
}

#[test]
fn parse_tag_based_no_parameters_block() {
    let content = r#"
<function_calls>
<invoke>
<tool_name>NoOp</tool_name>
</invoke>
</function_calls>
"#;
    let calls = parse_tool_calls(content, None, false);
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].function.name, Some("NoOp".to_string()));

    let args: serde_json::Value =
        serde_json::from_str(calls[0].function.arguments.as_ref().unwrap()).unwrap();
    assert!(args.as_object().unwrap().is_empty());
}

#[test]
fn parse_tag_based_empty_tool_name_skipped() {
    // <tool_name></tool_name> has empty body — extract_between_tags won't match (.+?)
    let content = r#"
<function_calls>
<invoke>
<tool_name></tool_name>
<parameters><x>1</x></parameters>
</invoke>
</function_calls>
"#;
    let calls = parse_tool_calls(content, None, false);
    assert!(calls.is_empty());
}

#[test]
fn parse_tag_based_missing_tool_name_skipped() {
    let content = r#"
<function_calls>
<invoke>
<parameters><x>1</x></parameters>
</invoke>
</function_calls>
"#;
    let calls = parse_tool_calls(content, None, false);
    assert!(calls.is_empty());
}

// ===========================================================================
// Attribute-based XML parsing tests
// ===========================================================================

#[test]
fn parse_single_xml_tool_call_attr_based() {
    let content = r#"<function_calls>
<invoke name="Bash">
<parameter name="command">ls -la</parameter>
</invoke>
</function_calls>"#;

    let tool_calls = parse_tool_calls(content, None, false);

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
fn parse_xml_with_multiple_parameters_attr_based() {
    let content = r#"<function_calls>
<invoke name="Bash">
<parameter name="command">git mv MISSING-FEATURES.md docs/</parameter>
<parameter name="description">Move file</parameter>
</invoke>
</function_calls>"#;

    let tool_calls = parse_tool_calls(content, None, false);

    assert_eq!(tool_calls.len(), 1);
    assert_eq!(tool_calls[0].function.name, Some("Bash".to_string()));

    let args: serde_json::Value =
        serde_json::from_str(tool_calls[0].function.arguments.as_ref().unwrap()).unwrap();
    assert_eq!(args["command"], "git mv MISSING-FEATURES.md docs/");
    assert_eq!(args["description"], "Move file");
}

#[test]
fn parse_multiple_xml_invokes_attr_based() {
    let content = r#"<function_calls>
<invoke name="Bash">
<parameter name="command">ls</parameter>
</invoke>
<invoke name="Grep">
<parameter name="pattern">test</parameter>
</invoke>
</function_calls>"#;

    let tool_calls = parse_tool_calls(content, None, false);

    assert_eq!(tool_calls.len(), 2);
    assert_eq!(tool_calls[0].function.name, Some("Bash".to_string()));
    assert_eq!(tool_calls[1].function.name, Some("Grep".to_string()));

    // Each has a unique ID
    assert_ne!(tool_calls[0].id, tool_calls[1].id);
}

#[test]
fn parse_xml_with_surrounding_text_attr_based() {
    let content = r#"Let me check that for you.

<function_calls>
<invoke name="Bash">
<parameter name="command">ls -la</parameter>
</invoke>
</function_calls>

I'll run that now."#;

    let tool_calls = parse_tool_calls(content, None, false);

    assert_eq!(tool_calls.len(), 1);
    assert_eq!(tool_calls[0].function.name, Some("Bash".to_string()));
}

#[test]
fn parse_xml_no_parameters_attr_based() {
    let content = r#"<function_calls><invoke name="NoOp"></invoke></function_calls>"#;

    let tool_calls = parse_tool_calls(content, None, false);

    assert_eq!(tool_calls.len(), 1);
    assert_eq!(tool_calls[0].function.name, Some("NoOp".to_string()));

    let args: serde_json::Value =
        serde_json::from_str(tool_calls[0].function.arguments.as_ref().unwrap()).unwrap();
    assert!(args.as_object().unwrap().is_empty());
}

#[test]
fn parse_xml_empty_name_returns_empty_attr_based() {
    let content = r#"<function_calls><invoke name=""><parameter name="x">y</parameter></invoke></function_calls>"#;

    let tool_calls = parse_tool_calls(content, None, false);
    assert!(tool_calls.is_empty());
}

#[test]
fn parse_xml_with_whitespace_in_tags_attr_based() {
    let content = r#"<function_calls>
  <invoke name="Bash">
    <parameter name="command">echo hello</parameter>
  </invoke>
</function_calls>"#;

    let tool_calls = parse_tool_calls(content, None, false);

    assert_eq!(tool_calls.len(), 1);
    assert_eq!(tool_calls[0].function.name, Some("Bash".to_string()));

    let args: serde_json::Value =
        serde_json::from_str(tool_calls[0].function.arguments.as_ref().unwrap()).unwrap();
    assert_eq!(args["command"], "echo hello");
}

#[test]
fn parse_xml_multiple_function_calls_blocks_attr_based() {
    let content = r#"First block:
<function_calls>
<invoke name="Tool1"><parameter name="a">1</parameter></invoke>
</function_calls>

Second block:
<function_calls>
<invoke name="Tool2"><parameter name="b">2</parameter></invoke>
</function_calls>"#;

    let tool_calls = parse_tool_calls(content, None, false);

    assert_eq!(tool_calls.len(), 2);
    assert_eq!(tool_calls[0].function.name, Some("Tool1".to_string()));
    assert_eq!(tool_calls[1].function.name, Some("Tool2".to_string()));
}

#[test]
fn parse_xml_parameters_are_all_strings() {
    let content = r#"<function_calls>
<invoke name="Test">
<parameter name="count">42</parameter>
<parameter name="active">true</parameter>
<parameter name="name">hello</parameter>
</invoke>
</function_calls>"#;

    let tool_calls = parse_tool_calls(content, None, false);
    assert_eq!(tool_calls.len(), 1);

    let args: serde_json::Value =
        serde_json::from_str(tool_calls[0].function.arguments.as_ref().unwrap()).unwrap();
    assert_eq!(args["count"], "42");
    assert_eq!(args["active"], "true");
    assert_eq!(args["name"], "hello");
}

#[test]
fn parse_xml_trims_parameter_whitespace_attr_based() {
    let content = r#"<function_calls>
<invoke name="Bash">
<parameter name="command">
  echo hello world
</parameter>
</invoke>
</function_calls>"#;

    let tool_calls = parse_tool_calls(content, None, false);
    assert_eq!(tool_calls.len(), 1);

    let args: serde_json::Value =
        serde_json::from_str(tool_calls[0].function.arguments.as_ref().unwrap()).unwrap();
    assert_eq!(args["command"], "echo hello world");
}

#[test]
fn parse_xml_trims_multiline_parameter_value_attr_based() {
    let content = r#"<function_calls>
<invoke name="WriteFile">
<parameter name="path">  /tmp/test.txt  </parameter>
<parameter name="content">   some content   </parameter>
</invoke>
</function_calls>"#;

    let tool_calls = parse_tool_calls(content, None, false);
    assert_eq!(tool_calls.len(), 1);

    let args: serde_json::Value =
        serde_json::from_str(tool_calls[0].function.arguments.as_ref().unwrap()).unwrap();
    assert_eq!(args["path"], "/tmp/test.txt");
    assert_eq!(args["content"], "some content");
}

// ===========================================================================
// Standalone invoke (fallback) tests
// ===========================================================================

#[test]
fn parse_standalone_invoke_attr_based() {
    let content = r#"<invoke name="Bash"><parameter name="command">ls</parameter></invoke>"#;

    let tool_calls = parse_tool_calls(content, None, false);
    assert_eq!(tool_calls.len(), 1);
    assert_eq!(tool_calls[0].function.name, Some("Bash".to_string()));
}

#[test]
fn parse_standalone_invoke_prefers_function_calls_wrapper() {
    // When <function_calls> is present, standalone invokes outside it are
    // NOT parsed (primary path returns early)
    let content = r#"<function_calls>
<invoke name="Inner"><parameter name="x">1</parameter></invoke>
</function_calls>
<invoke name="Outer"><parameter name="y">2</parameter></invoke>"#;

    let tool_calls = parse_tool_calls(content, None, false);
    assert_eq!(tool_calls.len(), 1);
    assert_eq!(tool_calls[0].function.name, Some("Inner".to_string()));
}

// ===========================================================================
// No tool calls / empty / plain text
// ===========================================================================

#[test]
fn no_tool_calls_returns_empty() {
    let tool_calls = parse_tool_calls("Just a regular message with no tool calls at all.", None, false);
    assert!(tool_calls.is_empty());
}

#[test]
fn empty_string_returns_empty() {
    let tool_calls = parse_tool_calls("", None, false);
    assert!(tool_calls.is_empty());
}

#[test]
fn no_content_returns_empty_for_various_inputs() {
    assert!(parse_tool_calls("", None, false).is_empty());
    assert!(parse_tool_calls("just plain text", None, false).is_empty());
    assert!(parse_tool_calls("<not_function_calls></not_function_calls>", None, false).is_empty());
}

#[test]
fn xml_empty_function_calls_returns_empty() {
    let content = "<function_calls></function_calls>";
    let tool_calls = parse_tool_calls(content, None, false);
    assert!(tool_calls.is_empty());
}

// ===========================================================================
// Unique ID tests
// ===========================================================================

#[test]
fn tool_call_ids_are_unique_across_calls() {
    let content = r#"<function_calls>
<invoke name="A"><parameter name="x">1</parameter></invoke>
<invoke name="B"><parameter name="y">2</parameter></invoke>
<invoke name="C"><parameter name="z">3</parameter></invoke>
</function_calls>"#;

    let tool_calls = parse_tool_calls(content, None, false);

    assert_eq!(tool_calls.len(), 3);
    let ids: Vec<&str> = tool_calls
        .iter()
        .map(|tc| tc.id.as_ref().unwrap().as_str())
        .collect();

    let mut unique_ids = ids.clone();
    unique_ids.sort();
    unique_ids.dedup();
    assert_eq!(ids.len(), unique_ids.len());

    for id in &ids {
        assert!(id.starts_with("call_"));
    }
}

#[test]
fn tag_based_tool_call_ids_are_unique() {
    let content = r#"<function_calls>
<invoke>
<tool_name>A</tool_name>
<parameters><x>1</x></parameters>
</invoke>
<invoke>
<tool_name>B</tool_name>
<parameters><y>2</y></parameters>
</invoke>
</function_calls>"#;

    let tool_calls = parse_tool_calls(content, None, false);
    assert_eq!(tool_calls.len(), 2);

    assert_ne!(tool_calls[0].id, tool_calls[1].id);
    assert!(tool_calls[0].id.as_ref().unwrap().starts_with("call_"));
    assert!(tool_calls[1].id.as_ref().unwrap().starts_with("call_"));
}

// ===========================================================================
// strip_tool_calls tests
// ===========================================================================

#[test]
fn strip_tool_calls_preserves_text() {
    let content =
        "Before\n<function_calls><invoke><tool_name>X</tool_name></invoke></function_calls>\nAfter";
    let stripped = strip_tool_calls(content);
    assert!(stripped.contains("Before"));
    assert!(stripped.contains("After"));
    assert!(!stripped.contains("function_calls"));
}

#[test]
fn strip_xml_tool_call_attr_based() {
    let content = r#"Let me check that.

<function_calls>
<invoke name="Bash">
<parameter name="command">ls -la</parameter>
</invoke>
</function_calls>

Done."#;

    let stripped = strip_tool_calls(content);
    assert!(!stripped.contains("<function_calls>"));
    assert!(!stripped.contains("<invoke"));
    assert!(stripped.contains("Let me check that."));
    assert!(stripped.contains("Done."));
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
fn strip_standalone_invoke() {
    let content = r#"Before.

<invoke name="Bash"><parameter name="command">pwd</parameter></invoke>

After."#;

    let stripped = strip_tool_calls(content);
    assert!(!stripped.contains("<invoke"));
    assert!(stripped.contains("Before."));
    assert!(stripped.contains("After."));
}

#[test]
fn strip_standalone_invoke_tag_based() {
    let content = r#"Before.

<invoke>
<tool_name>Read</tool_name>
<parameters><file_path>/a.rs</file_path></parameters>
</invoke>

After."#;

    let stripped = strip_tool_calls(content);
    assert!(!stripped.contains("<invoke>"));
    assert!(!stripped.contains("tool_name"));
    assert!(stripped.contains("Before."));
    assert!(stripped.contains("After."));
}

#[test]
fn strip_no_tool_calls_returns_original() {
    let content = "Just regular text with no tool calls.";
    let stripped = strip_tool_calls(content);
    assert_eq!(stripped, content);
}

#[test]
fn strip_preserves_regular_xml() {
    let content = "See: <note>Important</note>";
    let stripped = strip_tool_calls(content);
    assert!(stripped.contains("<note>"));
    assert!(stripped.contains("Important"));
    assert!(stripped.contains("</note>"));
}

#[test]
fn strip_collapses_excessive_blank_lines() {
    let content = r#"Before.



<function_calls>
<invoke name="test"><parameter name="x">1</parameter></invoke>
</function_calls>



After."#;

    let stripped = strip_tool_calls(content);
    assert!(!stripped.contains("\n\n\n"));
    assert!(stripped.contains("Before."));
    assert!(stripped.contains("After."));
}

#[test]
fn strip_tag_based_function_calls() {
    let content = r#"Text before.

<function_calls>
<invoke>
<tool_name>Read</tool_name>
<parameters>
<file_path>/src/main.rs</file_path>
</parameters>
</invoke>
</function_calls>

Text after."#;

    let stripped = strip_tool_calls(content);
    assert!(!stripped.contains("<function_calls>"));
    assert!(!stripped.contains("tool_name"));
    assert!(stripped.contains("Text before."));
    assert!(stripped.contains("Text after."));
}

// ===========================================================================
// Edge cases
// ===========================================================================

#[test]
fn parse_xml_malformed_missing_name_attribute() {
    // Attribute-based invoke without name attribute — falls through to tag-based
    // which also finds nothing since there's no <tool_name>
    let content =
        r#"<function_calls><invoke><parameter name="x">y</parameter></invoke></function_calls>"#;

    let tool_calls = parse_tool_calls(content, None, false);
    assert!(tool_calls.is_empty());
}

#[test]
fn regular_json_code_block_not_affected() {
    // JSON code blocks should not be parsed (no JSON parsing anymore)
    let content = r#"Here's some JSON:

```json
{"key": "value", "items": [1, 2, 3]}
```

That's a sample."#;

    let tool_calls = parse_tool_calls(content, None, false);
    assert!(tool_calls.is_empty());
}

#[test]
fn json_function_call_format_no_longer_parsed() {
    // JSON tool calls are not parsed anymore — only XML
    let content = r#"```json
{"function_call": {"name": "bash", "arguments": {"command": "ls"}}}
```"#;

    let tool_calls = parse_tool_calls(content, None, false);
    assert!(tool_calls.is_empty());
}

#[test]
fn strip_preserves_json_code_blocks() {
    // JSON code blocks should NOT be stripped (no JSON parsing)
    let content = r#"See the error format:

```json
{"error": "not found", "code": 404}
```

That block should remain."#;

    let stripped = strip_tool_calls(content);
    assert!(stripped.contains(r#"{"error": "not found", "code": 404}"#));
    assert!(stripped.contains("```json"));
}

#[test]
fn parse_tag_based_with_whitespace_in_params() {
    let content = r#"<function_calls>
<invoke>
<tool_name>WriteFile</tool_name>
<parameters>
<path>  /tmp/test.txt  </path>
<content>   hello world   </content>
</parameters>
</invoke>
</function_calls>"#;

    let calls = parse_tool_calls(content, None, false);
    assert_eq!(calls.len(), 1);

    let args: serde_json::Value =
        serde_json::from_str(calls[0].function.arguments.as_ref().unwrap()).unwrap();
    assert_eq!(args["path"], "/tmp/test.txt");
    assert_eq!(args["content"], "hello world");
}

#[test]
fn parse_tag_based_multiple_function_calls_blocks() {
    let content = r#"First:
<function_calls>
<invoke>
<tool_name>Tool1</tool_name>
<parameters><a>1</a></parameters>
</invoke>
</function_calls>

Second:
<function_calls>
<invoke>
<tool_name>Tool2</tool_name>
<parameters><b>2</b></parameters>
</invoke>
</function_calls>"#;

    let calls = parse_tool_calls(content, None, false);
    assert_eq!(calls.len(), 2);
    assert_eq!(calls[0].function.name, Some("Tool1".to_string()));
    assert_eq!(calls[1].function.name, Some("Tool2".to_string()));
}

// ===========================================================================
// Parameter values containing '<' (Bug 1 regression test)
// ===========================================================================

#[test]
fn parse_attr_parameter_with_less_than_sign() {
    let content = r#"<function_calls>
<invoke name="Bash">
<parameter name="command">echo x < 10</parameter>
</invoke>
</function_calls>"#;

    let tool_calls = parse_tool_calls(content, None, false);
    assert_eq!(tool_calls.len(), 1);

    let args: serde_json::Value =
        serde_json::from_str(tool_calls[0].function.arguments.as_ref().unwrap()).unwrap();
    assert_eq!(args["command"], "echo x < 10");
}

#[test]
fn parse_attr_parameter_with_html_content() {
    let content = r#"<function_calls>
<invoke name="WriteFile">
<parameter name="path">/tmp/test.html</parameter>
<parameter name="content"><div><p>hello</p></div></parameter>
</invoke>
</function_calls>"#;

    let tool_calls = parse_tool_calls(content, None, false);
    assert_eq!(tool_calls.len(), 1);

    let args: serde_json::Value =
        serde_json::from_str(tool_calls[0].function.arguments.as_ref().unwrap()).unwrap();
    assert_eq!(args["path"], "/tmp/test.html");
    assert_eq!(args["content"], "<div><p>hello</p></div>");
}

#[test]
fn parse_attr_parameter_with_comparison_operator() {
    let content = r#"<function_calls>
<invoke name="Bash">
<parameter name="command">if x < 10 && y > 5; then echo ok; fi</parameter>
</invoke>
</function_calls>"#;

    let tool_calls = parse_tool_calls(content, None, false);
    assert_eq!(tool_calls.len(), 1);

    let args: serde_json::Value =
        serde_json::from_str(tool_calls[0].function.arguments.as_ref().unwrap()).unwrap();
    assert_eq!(
        args["command"],
        "if x < 10 && y > 5; then echo ok; fi"
    );
}
