# Bug Analysis: Tool Parameter Type Information Lost

## Summary

The copilot-adapter loses type information when parsing tool calls from XML, converting all parameter values to strings regardless of their JSON schema types. This causes validation errors when Claude Code receives tool calls with incorrect types.

## Reproduction

1. Claude Code sends a request with MCP tool definitions that have typed parameters:
   - `limit: number`
   - `includeSnippet: boolean`

2. The model responds with XML containing tool calls:
   ```xml
   <invoke name="mcp__codemogger__codemogger_search">
   <parameter name="limit">10</parameter>
   <parameter name="includeSnippet">true</parameter>
   </invoke>
   ```

3. The adapter parses this and returns:
   ```json
   {
     "type": "tool_use",
     "id": "call_64420623b459",
     "name": "mcp__codemogger__codemogger_search",
     "input": {
       "limit": "10",        // ← Should be number 10
       "includeSnippet": "true"  // ← Should be boolean true
     }
   }
   ```

4. Claude Code validates the tool call against the MCP schema and rejects it:
   ```
   MCP error -32602: Input validation error: Invalid arguments for tool codemogger_search: [
     {
       "expected": "number",
       "code": "invalid_type",
       "path": ["limit"],
       "message": "Invalid input: expected number, received string"
     },
     {
       "expected": "boolean",
       "code": "invalid_type",
       "path": ["includeSnippet"],
       "message": "Invalid input: expected boolean, received string"
     }
   ]
   ```

## Root Cause

The XML parser in `src/tools/parser.rs` always converts parameter values to `serde_json::Value::String`:

### Tag-based format (lines 116-138):
```rust
fn parse_xml_params(params_content: &str) -> serde_json::Value {
    let mut params = serde_json::Map::new();

    for cap in OPEN_TAG.captures_iter(params_content) {
        // ... extract tag and value ...
        params.insert(
            name.to_string(),
            serde_json::Value::String(value.trim().to_string()),  // ← ALWAYS String
        );
    }

    serde_json::Value::Object(params)
}
```

### Attribute-based format (lines 144-157):
```rust
fn parse_attribute_params(invoke_body: &str) -> serde_json::Value {
    let mut params = serde_json::Map::new();

    for param_cap in XML_PARAMETER.captures_iter(invoke_body) {
        let param_name = param_cap.get(1).unwrap().as_str();
        let param_value = param_cap.get(2).unwrap().as_str().trim();
        params.insert(
            param_name.to_string(),
            serde_json::Value::String(param_value.to_string()),  // ← ALWAYS String
        );
    }

    serde_json::Value::Object(params)
}
```

## Why This Happens

1. **XML is untyped**: XML tags contain text content only. There's no way to distinguish `<limit>10</limit>` (number) from `<name>10</name>` (string) without external schema information.

2. **Parser has no schema context**: The parser functions operate on raw XML strings and don't have access to the tool definitions or their JSON schemas that were sent in the original request.

3. **Type inference is ambiguous**: While `"10"` could be coerced to a number and `"true"` to a boolean, strings like `"3.14"`, `"true"`, `"null"` could be either literal strings or typed values depending on context.

## Evidence from Logs

From `logs2.txt` line 1281:
```xml
<parameter name="limit">10</parameter>
```

Parsed to (line 1330):
```json
"limit": "10"
```

Expected by MCP schema (line 1382):
```json
{
  "expected": "number",
  "code": "invalid_type",
  "path": ["limit"],
  "message": "Invalid input: expected number, received string"
}
```

## Why Claude's Attempted Fix Didn't Work

From `claude_log2.txt` line 52-68, Claude Code tried to fix the issue:
```
● Let me fix the codemogger search calls with correct parameter types:

● codemogger - codemogger_search (MCP)(includeSnippet: "true", limit:
                                      "10", mode: "semantic", query:
                                      "stop button UI handler")
```

The adapter still parsed it as strings because:
1. The model still emits `<parameter name="limit">10</parameter>` (text content)
2. The parser has no way to know that `limit` should be a number
3. The type information exists only in the tool schema, not in the XML response

## Solution Approaches

### Option 1: Type Inference (Risky)
Parse parameter values as JSON primitives when they match known patterns:
- `"10"` → `10` (number)
- `"true"`/`"false"` → boolean
- `"null"` → null
- Otherwise → string

**Pros**: No schema lookup needed
**Cons**: Breaks if a tool legitimately expects string `"10"` or `"true"`

### Option 2: Schema-Aware Parsing (Correct but Complex)
1. Store tool definitions from the request in the handler context
2. Pass them to the parser
3. Look up each parameter's expected type and parse accordingly

**Pros**: Correct behavior, respects schema
**Cons**: Requires passing context through multiple layers

### Option 3: JSON in XML (Hybrid)
Instruct the model to emit JSON values in XML:
```xml
<parameter name="limit" type="number">10</parameter>
<parameter name="includeSnippet" type="boolean">true</parameter>
```

**Pros**: Self-documenting, no schema lookup
**Cons**: Requires prompt changes, model may not follow consistently

### Option 4: Pure JSON Tool Format (Breaking Change)
Switch from XML to JSON tool calls entirely:
```json
{"tool_calls": [{"name": "...", "arguments": {"limit": 10, "includeSnippet": true}}]}
```

**Pros**: Native type preservation
**Cons**: Major design change, requires updating injector and parser

## Recommended Fix

**Option 2 (Schema-Aware Parsing)** is the most correct solution:

1. Update handler to pass tool definitions to parser
2. Create a `ToolRegistry` that indexes tools by name and parameters by name
3. Update `parse_attribute_params` and `parse_xml_params` to:
   - Look up the tool and parameter in the registry
   - Parse the string value according to the schema type
   - Fall back to string if tool/param not found

Implementation sketch:
```rust
fn parse_attribute_params(
    invoke_body: &str,
    tool_name: &str,
    tool_registry: &ToolRegistry,
) -> serde_json::Value {
    let mut params = serde_json::Map::new();

    for param_cap in XML_PARAMETER.captures_iter(invoke_body) {
        let param_name = param_cap.get(1).unwrap().as_str();
        let param_value = param_cap.get(2).unwrap().as_str().trim();

        // Look up expected type from schema
        let typed_value = if let Some(param_type) = tool_registry.get_param_type(tool_name, param_name) {
            parse_value_with_type(param_value, param_type)
        } else {
            serde_json::Value::String(param_value.to_string())
        };

        params.insert(param_name.to_string(), typed_value);
    }

    serde_json::Value::Object(params)
}

fn parse_value_with_type(value: &str, schema_type: &str) -> serde_json::Value {
    match schema_type {
        "number" | "integer" => {
            value.parse::<f64>()
                .map(serde_json::Value::from)
                .unwrap_or_else(|_| serde_json::Value::String(value.to_string()))
        }
        "boolean" => {
            match value {
                "true" => serde_json::Value::Bool(true),
                "false" => serde_json::Value::Bool(false),
                _ => serde_json::Value::String(value.to_string()),
            }
        }
        "null" => {
            if value == "null" {
                serde_json::Value::Null
            } else {
                serde_json::Value::String(value.to_string())
            }
        }
        "object" | "array" => {
            // Try to parse as JSON
            serde_json::from_str(value)
                .unwrap_or_else(|_| serde_json::Value::String(value.to_string()))
        }
        _ => serde_json::Value::String(value.to_string()),
    }
}
```

## Files Involved

- `src/tools/parser.rs` - XML parser that needs schema awareness
- `src/handlers/messages.rs` - Handler that receives tool definitions and calls parser
- `src/anthropic/types.rs` - Contains `ToolDefinition` and `InputSchema` types

## Impact

**Current behavior**: MCP tools with non-string parameters don't work through copilot-adapter

**After fix**: All parameter types will be correctly preserved and validated by Claude Code
