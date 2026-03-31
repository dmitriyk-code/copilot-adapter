use copilot_adapter::anthropic::types::{InputSchema, ToolDefinition};
use copilot_adapter::tools::parser::parse_tool_calls;
use copilot_adapter::tools::registry::{parse_value_with_type, ParamType, ToolRegistry};

// ===========================================================================
// Helper: build ToolDefinition from a JSON properties object
// ===========================================================================

fn make_tool(name: &str, properties: serde_json::Value) -> ToolDefinition {
    ToolDefinition {
        name: name.to_string(),
        description: Some(format!("{name} tool")),
        input_schema: InputSchema {
            schema_type: "object".to_string(),
            properties: Some(properties),
            required: None,
        },
    }
}

fn make_registry(tools: &[ToolDefinition]) -> ToolRegistry {
    ToolRegistry::from_tools(tools)
}

// ===========================================================================
// E5-T8: Number parameter parsing
// ===========================================================================

#[test]
fn tag_based_number_param_coerced_to_number() {
    let tools = vec![make_tool(
        "Search",
        serde_json::json!({
            "query": {"type": "string"},
            "limit": {"type": "integer"},
            "threshold": {"type": "number"}
        }),
    )];
    let registry = make_registry(&tools);

    let content = r#"
<function_calls>
<invoke>
<tool_name>Search</tool_name>
<parameters>
<query>rust async</query>
<limit>10</limit>
<threshold>0.75</threshold>
</parameters>
</invoke>
</function_calls>
"#;
    let calls = parse_tool_calls(content, Some(&registry), false);
    assert_eq!(calls.len(), 1);

    let args: serde_json::Value =
        serde_json::from_str(calls[0].function.arguments.as_ref().unwrap()).unwrap();
    assert_eq!(args["query"], serde_json::json!("rust async"));
    assert_eq!(args["limit"], serde_json::json!(10));
    assert_eq!(args["threshold"], serde_json::json!(0.75));
}

#[test]
fn attr_based_number_param_coerced_to_number() {
    let tools = vec![make_tool(
        "Search",
        serde_json::json!({
            "query": {"type": "string"},
            "limit": {"type": "integer"}
        }),
    )];
    let registry = make_registry(&tools);

    let content = r#"
<function_calls>
<invoke name="Search">
<parameter name="query">test</parameter>
<parameter name="limit">42</parameter>
</invoke>
</function_calls>
"#;
    let calls = parse_tool_calls(content, Some(&registry), false);
    assert_eq!(calls.len(), 1);

    let args: serde_json::Value =
        serde_json::from_str(calls[0].function.arguments.as_ref().unwrap()).unwrap();
    assert_eq!(args["query"], serde_json::json!("test"));
    assert_eq!(args["limit"], serde_json::json!(42));
}

#[test]
fn negative_number_coerced() {
    let tools = vec![make_tool(
        "Adjust",
        serde_json::json!({
            "offset": {"type": "integer"},
            "scale": {"type": "number"}
        }),
    )];
    let registry = make_registry(&tools);

    let content = r#"
<function_calls>
<invoke>
<tool_name>Adjust</tool_name>
<parameters>
<offset>-5</offset>
<scale>-0.5</scale>
</parameters>
</invoke>
</function_calls>
"#;
    let calls = parse_tool_calls(content, Some(&registry), false);
    let args: serde_json::Value =
        serde_json::from_str(calls[0].function.arguments.as_ref().unwrap()).unwrap();
    assert_eq!(args["offset"], serde_json::json!(-5));
    assert_eq!(args["scale"], serde_json::json!(-0.5));
}

#[test]
fn invalid_number_falls_back_to_string() {
    let tools = vec![make_tool(
        "Calc",
        serde_json::json!({
            "value": {"type": "integer"}
        }),
    )];
    let registry = make_registry(&tools);

    let content = r#"
<function_calls>
<invoke>
<tool_name>Calc</tool_name>
<parameters>
<value>not_a_number</value>
</parameters>
</invoke>
</function_calls>
"#;
    let calls = parse_tool_calls(content, Some(&registry), false);
    let args: serde_json::Value =
        serde_json::from_str(calls[0].function.arguments.as_ref().unwrap()).unwrap();
    assert_eq!(args["value"], serde_json::json!("not_a_number"));
}

// ===========================================================================
// E5-T9: Boolean parameter parsing
// ===========================================================================

#[test]
fn boolean_param_coerced_true() {
    let tools = vec![make_tool(
        "Toggle",
        serde_json::json!({
            "enabled": {"type": "boolean"}
        }),
    )];
    let registry = make_registry(&tools);

    let content = r#"
<function_calls>
<invoke>
<tool_name>Toggle</tool_name>
<parameters>
<enabled>true</enabled>
</parameters>
</invoke>
</function_calls>
"#;
    let calls = parse_tool_calls(content, Some(&registry), false);
    let args: serde_json::Value =
        serde_json::from_str(calls[0].function.arguments.as_ref().unwrap()).unwrap();
    assert_eq!(args["enabled"], serde_json::json!(true));
}

#[test]
fn boolean_param_coerced_false() {
    let tools = vec![make_tool(
        "Toggle",
        serde_json::json!({
            "verbose": {"type": "boolean"}
        }),
    )];
    let registry = make_registry(&tools);

    let content = r#"
<function_calls>
<invoke name="Toggle">
<parameter name="verbose">false</parameter>
</invoke>
</function_calls>
"#;
    let calls = parse_tool_calls(content, Some(&registry), false);
    let args: serde_json::Value =
        serde_json::from_str(calls[0].function.arguments.as_ref().unwrap()).unwrap();
    assert_eq!(args["verbose"], serde_json::json!(false));
}

#[test]
fn boolean_param_case_insensitive() {
    let tools = vec![make_tool(
        "Toggle",
        serde_json::json!({
            "flag": {"type": "boolean"}
        }),
    )];
    let registry = make_registry(&tools);

    let content = r#"
<function_calls>
<invoke>
<tool_name>Toggle</tool_name>
<parameters>
<flag>TRUE</flag>
</parameters>
</invoke>
</function_calls>
"#;
    let calls = parse_tool_calls(content, Some(&registry), false);
    let args: serde_json::Value =
        serde_json::from_str(calls[0].function.arguments.as_ref().unwrap()).unwrap();
    assert_eq!(args["flag"], serde_json::json!(true));
}

#[test]
fn boolean_param_invalid_falls_back_to_string() {
    let tools = vec![make_tool(
        "Toggle",
        serde_json::json!({
            "flag": {"type": "boolean"}
        }),
    )];
    let registry = make_registry(&tools);

    let content = r#"
<function_calls>
<invoke>
<tool_name>Toggle</tool_name>
<parameters>
<flag>yes</flag>
</parameters>
</invoke>
</function_calls>
"#;
    let calls = parse_tool_calls(content, Some(&registry), false);
    let args: serde_json::Value =
        serde_json::from_str(calls[0].function.arguments.as_ref().unwrap()).unwrap();
    assert_eq!(args["flag"], serde_json::json!("yes"));
}

// ===========================================================================
// E5-T10: Object parameter parsing
// ===========================================================================

#[test]
fn object_param_coerced_to_json_object() {
    let tools = vec![make_tool(
        "Configure",
        serde_json::json!({
            "settings": {"type": "object"}
        }),
    )];
    let registry = make_registry(&tools);

    let content = r#"
<function_calls>
<invoke>
<tool_name>Configure</tool_name>
<parameters>
<settings>{"timeout": 30, "retries": 3}</settings>
</parameters>
</invoke>
</function_calls>
"#;
    let calls = parse_tool_calls(content, Some(&registry), false);
    let args: serde_json::Value =
        serde_json::from_str(calls[0].function.arguments.as_ref().unwrap()).unwrap();
    assert_eq!(
        args["settings"],
        serde_json::json!({"timeout": 30, "retries": 3})
    );
}

#[test]
fn nested_object_param_coerced() {
    let tools = vec![make_tool(
        "Deploy",
        serde_json::json!({
            "config": {"type": "object"}
        }),
    )];
    let registry = make_registry(&tools);

    let content = r#"
<function_calls>
<invoke name="Deploy">
<parameter name="config">{"env": {"NODE_ENV": "prod"}, "port": 8080}</parameter>
</invoke>
</function_calls>
"#;
    let calls = parse_tool_calls(content, Some(&registry), false);
    let args: serde_json::Value =
        serde_json::from_str(calls[0].function.arguments.as_ref().unwrap()).unwrap();
    assert_eq!(
        args["config"],
        serde_json::json!({"env": {"NODE_ENV": "prod"}, "port": 8080})
    );
}

#[test]
fn object_param_invalid_json_falls_back_to_string() {
    let tools = vec![make_tool(
        "Configure",
        serde_json::json!({
            "settings": {"type": "object"}
        }),
    )];
    let registry = make_registry(&tools);

    let content = r#"
<function_calls>
<invoke>
<tool_name>Configure</tool_name>
<parameters>
<settings>not valid json</settings>
</parameters>
</invoke>
</function_calls>
"#;
    let calls = parse_tool_calls(content, Some(&registry), false);
    let args: serde_json::Value =
        serde_json::from_str(calls[0].function.arguments.as_ref().unwrap()).unwrap();
    assert_eq!(args["settings"], serde_json::json!("not valid json"));
}

// ===========================================================================
// E5-T11: Array parameter parsing
// ===========================================================================

#[test]
fn array_param_coerced_to_json_array() {
    let tools = vec![make_tool(
        "BatchProcess",
        serde_json::json!({
            "items": {"type": "array"}
        }),
    )];
    let registry = make_registry(&tools);

    let content = r#"
<function_calls>
<invoke>
<tool_name>BatchProcess</tool_name>
<parameters>
<items>[1, 2, 3]</items>
</parameters>
</invoke>
</function_calls>
"#;
    let calls = parse_tool_calls(content, Some(&registry), false);
    let args: serde_json::Value =
        serde_json::from_str(calls[0].function.arguments.as_ref().unwrap()).unwrap();
    assert_eq!(args["items"], serde_json::json!([1, 2, 3]));
}

#[test]
fn string_array_param_coerced() {
    let tools = vec![make_tool(
        "Filter",
        serde_json::json!({
            "tags": {"type": "array"}
        }),
    )];
    let registry = make_registry(&tools);

    let content = r#"
<function_calls>
<invoke name="Filter">
<parameter name="tags">["rust", "async", "tokio"]</parameter>
</invoke>
</function_calls>
"#;
    let calls = parse_tool_calls(content, Some(&registry), false);
    let args: serde_json::Value =
        serde_json::from_str(calls[0].function.arguments.as_ref().unwrap()).unwrap();
    assert_eq!(args["tags"], serde_json::json!(["rust", "async", "tokio"]));
}

#[test]
fn array_param_invalid_json_falls_back_to_string() {
    let tools = vec![make_tool(
        "BatchProcess",
        serde_json::json!({
            "items": {"type": "array"}
        }),
    )];
    let registry = make_registry(&tools);

    let content = r#"
<function_calls>
<invoke>
<tool_name>BatchProcess</tool_name>
<parameters>
<items>not an array</items>
</parameters>
</invoke>
</function_calls>
"#;
    let calls = parse_tool_calls(content, Some(&registry), false);
    let args: serde_json::Value =
        serde_json::from_str(calls[0].function.arguments.as_ref().unwrap()).unwrap();
    assert_eq!(args["items"], serde_json::json!("not an array"));
}

// ===========================================================================
// E5-T12: Fallback to string on unknown tool/param
// ===========================================================================

#[test]
fn unknown_tool_falls_back_to_string() {
    let tools = vec![make_tool(
        "KnownTool",
        serde_json::json!({
            "count": {"type": "integer"}
        }),
    )];
    let registry = make_registry(&tools);

    let content = r#"
<function_calls>
<invoke>
<tool_name>UnknownTool</tool_name>
<parameters>
<count>42</count>
</parameters>
</invoke>
</function_calls>
"#;
    let calls = parse_tool_calls(content, Some(&registry), false);
    let args: serde_json::Value =
        serde_json::from_str(calls[0].function.arguments.as_ref().unwrap()).unwrap();
    // Unknown tool: "42" stays as string
    assert_eq!(args["count"], serde_json::json!("42"));
}

#[test]
fn unknown_param_falls_back_to_string() {
    let tools = vec![make_tool(
        "Search",
        serde_json::json!({
            "query": {"type": "string"}
        }),
    )];
    let registry = make_registry(&tools);

    let content = r#"
<function_calls>
<invoke>
<tool_name>Search</tool_name>
<parameters>
<query>test</query>
<unknown_param>42</unknown_param>
</parameters>
</invoke>
</function_calls>
"#;
    let calls = parse_tool_calls(content, Some(&registry), false);
    let args: serde_json::Value =
        serde_json::from_str(calls[0].function.arguments.as_ref().unwrap()).unwrap();
    assert_eq!(args["query"], serde_json::json!("test"));
    // Unknown param: stays as string
    assert_eq!(args["unknown_param"], serde_json::json!("42"));
}

#[test]
fn no_registry_all_params_are_strings() {
    let content = r#"
<function_calls>
<invoke>
<tool_name>Search</tool_name>
<parameters>
<query>test</query>
<limit>10</limit>
<fuzzy>true</fuzzy>
</parameters>
</invoke>
</function_calls>
"#;
    let calls = parse_tool_calls(content, None, false);
    let args: serde_json::Value =
        serde_json::from_str(calls[0].function.arguments.as_ref().unwrap()).unwrap();
    // Without registry, everything is a string
    assert_eq!(args["query"], serde_json::json!("test"));
    assert_eq!(args["limit"], serde_json::json!("10"));
    assert_eq!(args["fuzzy"], serde_json::json!("true"));
}

// ===========================================================================
// Mixed-type tool calls
// ===========================================================================

#[test]
fn mixed_types_all_coerced_correctly() {
    let tools = vec![make_tool(
        "ComplexTool",
        serde_json::json!({
            "name": {"type": "string"},
            "count": {"type": "integer"},
            "ratio": {"type": "number"},
            "enabled": {"type": "boolean"},
            "config": {"type": "object"},
            "tags": {"type": "array"}
        }),
    )];
    let registry = make_registry(&tools);

    let content = r#"
<function_calls>
<invoke>
<tool_name>ComplexTool</tool_name>
<parameters>
<name>test</name>
<count>5</count>
<ratio>3.14</ratio>
<enabled>true</enabled>
<config>{"key": "val"}</config>
<tags>["a", "b"]</tags>
</parameters>
</invoke>
</function_calls>
"#;
    let calls = parse_tool_calls(content, Some(&registry), false);
    let args: serde_json::Value =
        serde_json::from_str(calls[0].function.arguments.as_ref().unwrap()).unwrap();

    assert_eq!(args["name"], serde_json::json!("test"));
    assert_eq!(args["count"], serde_json::json!(5));
    assert_eq!(args["ratio"], serde_json::json!(3.14));
    assert_eq!(args["enabled"], serde_json::json!(true));
    assert_eq!(args["config"], serde_json::json!({"key": "val"}));
    assert_eq!(args["tags"], serde_json::json!(["a", "b"]));
}

#[test]
fn multiple_tools_in_registry() {
    let tools = vec![
        make_tool(
            "Read",
            serde_json::json!({
                "path": {"type": "string"},
                "line": {"type": "integer"}
            }),
        ),
        make_tool(
            "Write",
            serde_json::json!({
                "path": {"type": "string"},
                "content": {"type": "string"},
                "append": {"type": "boolean"}
            }),
        ),
    ];
    let registry = make_registry(&tools);

    let content = r#"
<function_calls>
<invoke>
<tool_name>Read</tool_name>
<parameters>
<path>/main.rs</path>
<line>42</line>
</parameters>
</invoke>
<invoke>
<tool_name>Write</tool_name>
<parameters>
<path>/out.txt</path>
<content>hello</content>
<append>true</append>
</parameters>
</invoke>
</function_calls>
"#;
    let calls = parse_tool_calls(content, Some(&registry), false);
    assert_eq!(calls.len(), 2);

    let args0: serde_json::Value =
        serde_json::from_str(calls[0].function.arguments.as_ref().unwrap()).unwrap();
    assert_eq!(args0["path"], serde_json::json!("/main.rs"));
    assert_eq!(args0["line"], serde_json::json!(42));

    let args1: serde_json::Value =
        serde_json::from_str(calls[1].function.arguments.as_ref().unwrap()).unwrap();
    assert_eq!(args1["path"], serde_json::json!("/out.txt"));
    assert_eq!(args1["content"], serde_json::json!("hello"));
    assert_eq!(args1["append"], serde_json::json!(true));
}

// ===========================================================================
// parse_value_with_type direct tests
// ===========================================================================

#[test]
fn parse_value_number_zero() {
    assert_eq!(
        parse_value_with_type("0", &ParamType::Number),
        serde_json::json!(0.0)
    );
}

#[test]
fn parse_value_number_scientific() {
    assert_eq!(
        parse_value_with_type("1.5e10", &ParamType::Number),
        serde_json::json!(1.5e10)
    );
}

#[test]
fn parse_value_integer_large() {
    assert_eq!(
        parse_value_with_type("9999999999", &ParamType::Integer),
        serde_json::json!(9999999999i64)
    );
}

#[test]
fn parse_value_empty_string_number_falls_back() {
    assert_eq!(
        parse_value_with_type("", &ParamType::Number),
        serde_json::json!("")
    );
}

#[test]
fn parse_value_empty_string_boolean_falls_back() {
    assert_eq!(
        parse_value_with_type("", &ParamType::Boolean),
        serde_json::json!("")
    );
}

#[test]
fn parse_value_object_empty() {
    assert_eq!(
        parse_value_with_type("{}", &ParamType::Object),
        serde_json::json!({})
    );
}

#[test]
fn parse_value_array_empty() {
    assert_eq!(
        parse_value_with_type("[]", &ParamType::Array),
        serde_json::json!([])
    );
}

// ===========================================================================
// Cross-type JSON coercion mismatch tests
// ===========================================================================

#[test]
fn object_type_rejects_number_json() {
    // A valid JSON number should NOT be accepted as an object
    assert_eq!(
        parse_value_with_type("42", &ParamType::Object),
        serde_json::json!("42")
    );
}

#[test]
fn object_type_rejects_array_json() {
    // A valid JSON array should NOT be accepted as an object
    assert_eq!(
        parse_value_with_type("[1,2,3]", &ParamType::Object),
        serde_json::json!("[1,2,3]")
    );
}

#[test]
fn object_type_rejects_boolean_json() {
    // A valid JSON boolean should NOT be accepted as an object
    assert_eq!(
        parse_value_with_type("true", &ParamType::Object),
        serde_json::json!("true")
    );
}

#[test]
fn object_type_rejects_null_json() {
    // A valid JSON null should NOT be accepted as an object
    assert_eq!(
        parse_value_with_type("null", &ParamType::Object),
        serde_json::json!("null")
    );
}

#[test]
fn object_type_rejects_string_json() {
    // A valid JSON string should NOT be accepted as an object
    assert_eq!(
        parse_value_with_type(r#""hello""#, &ParamType::Object),
        serde_json::json!(r#""hello""#)
    );
}

#[test]
fn array_type_rejects_boolean_json() {
    // A valid JSON boolean should NOT be accepted as an array
    assert_eq!(
        parse_value_with_type("true", &ParamType::Array),
        serde_json::json!("true")
    );
}

#[test]
fn array_type_rejects_number_json() {
    // A valid JSON number should NOT be accepted as an array
    assert_eq!(
        parse_value_with_type("42", &ParamType::Array),
        serde_json::json!("42")
    );
}

#[test]
fn array_type_rejects_object_json() {
    // A valid JSON object should NOT be accepted as an array
    assert_eq!(
        parse_value_with_type(r#"{"key":"val"}"#, &ParamType::Array),
        serde_json::json!(r#"{"key":"val"}"#)
    );
}

#[test]
fn array_type_rejects_null_json() {
    // A valid JSON null should NOT be accepted as an array
    assert_eq!(
        parse_value_with_type("null", &ParamType::Array),
        serde_json::json!("null")
    );
}
