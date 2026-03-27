# Tools/Functions Support for Copilot Adapter — Design Document (Draft)

**Status:** Draft / Research
**Date:** 2026-03-26
**Prerequisite:** Core adapter implementation (IMPLEMENTATION.plan.md)

---

## Context

The current Copilot Adapter design marks `tools` and `functions` parameters as "Not supported — Copilot limitation". However, users may still want tool/function calling capabilities when using Claude Code through the adapter. This document explores options for adding tool support via prompt injection, similar to how LiteLLM handles non-native function calling.

---

## Problem Statement

GitHub Copilot's API does not natively support OpenAI's `tools` or `functions` parameters. When a client sends a request with these parameters, the adapter currently has no mechanism to handle them, resulting in either:
1. Silently ignoring the parameters (confusing)
2. Returning an error (limiting functionality)

Users who rely on tool calling for agentic workflows would be unable to use those workflows through the adapter.

---

## Research: How LiteLLM Handles Non-Native Function Calling

LiteLLM provides a reference implementation for adding function calling to models that don't natively support it.

### Detection

```python
# LiteLLM capability detection
litellm.supports_function_calling(model="gpt-4")      # True
litellm.supports_function_calling(model="claude-2")   # False (at the time)
```

### Activation

```python
# Enable prompt injection for unsupported models
litellm.add_function_to_prompt = True
```

### Implementation Flow

1. **Check native support** — If model supports tools natively, pass through unchanged
2. **Inject into prompt** — Transform tool definitions into text appended to system prompt
3. **Send request** — Normal completion request (tools removed from API params)
4. **Parse response** — Extract function calls from model's text output
5. **Return structured response** — Convert parsed calls back to OpenAI tool_calls format

### Prompt Injection Format (XML for Claude)

LiteLLM uses `construct_format_tool_for_claude_prompt()`:

```xml
<tool_description>
<tool_name>get_weather</tool_name>
<description>
Get the current weather for a location
</description>
<parameters>
<parameter>
<name>location</name>
<type>string</type>
<description>City and state, e.g. "San Francisco, CA"</description>
<required>true</required>
</parameter>
<parameter>
<name>unit</name>
<type>string</type>
<enum>celsius, fahrenheit</enum>
<required>false</required>
</parameter>
</parameters>
</tool_description>
```

### System Prompt Template

The injected prompt typically includes:

```
You have access to the following tools:

<tools>
{tool_descriptions}
</tools>

To use a tool, respond with:
<tool_use>
<tool_name>function_name</tool_name>
<parameters>
<param_name>value</param_name>
</parameters>
</tool_use>

If you need to call multiple tools, include multiple <tool_use> blocks.
```

### Response Parsing

LiteLLM parses the model's text response looking for:
- XML tags: `<tool_use>`, `<tool_name>`, `<parameters>`
- JSON blocks (some models respond with JSON)
- Function call patterns in text

Parsed calls are converted back to OpenAI's `tool_calls` format:

```python
{
    "id": "call_abc123",
    "type": "function",
    "function": {
        "name": "get_weather",
        "arguments": "{\"location\": \"San Francisco, CA\"}"
    }
}
```

---

## Proposed Design Options

### Option A: No Support (Current Design)

**Behavior:** Return 400 error when `tools` or `functions` present in request.

**Pros:**
- Simple, no additional complexity
- No false expectations about reliability
- No parsing edge cases

**Cons:**
- Users with tool-dependent workflows cannot use the adapter
- Feature gap compared to LiteLLM and similar proxies

### Option B: Prompt Injection with XML Format

**Behavior:** Inject tool definitions into system prompt, parse XML responses.

**Implementation:**

```rust
// In copilot/client.rs

fn inject_tools_into_prompt(
    messages: &mut Vec<Message>,
    tools: &[Tool],
) {
    let tool_prompt = format_tools_as_xml(tools);

    // Prepend to system message or create one
    if let Some(system_msg) = messages.iter_mut().find(|m| m.role == "system") {
        system_msg.content = format!("{}\n\n{}", tool_prompt, system_msg.content);
    } else {
        messages.insert(0, Message {
            role: "system".to_string(),
            content: tool_prompt,
        });
    }
}

fn parse_tool_calls_from_response(content: &str) -> Vec<ToolCall> {
    // Parse <tool_use> XML blocks from response
    // Return structured ToolCall objects
}
```

**Pros:**
- Enables tool workflows for users
- XML format well-suited for GPT-4 (Copilot's backend)
- Follows LiteLLM's proven approach

**Cons:**
- Parsing is fragile — models don't always follow format
- Adds latency for parsing
- May confuse models not trained for XML tool format
- Increases token usage (tool definitions in prompt)

### Option C: Prompt Injection with JSON Format

**Behavior:** Similar to Option B, but use JSON format which GPT-4 handles well.

**Tool Prompt Template:**

```
You have access to these functions:

{
  "functions": [
    {
      "name": "get_weather",
      "description": "Get current weather",
      "parameters": {
        "type": "object",
        "properties": {
          "location": {"type": "string"}
        }
      }
    }
  ]
}

To call a function, respond with JSON:
{"function_call": {"name": "function_name", "arguments": {"arg": "value"}}}
```

**Pros:**
- GPT-4 is very good at generating valid JSON
- Easier to parse than XML
- Matches OpenAI's native format conceptually

**Cons:**
- Same reliability concerns as XML
- JSON in prompts can confuse some edge cases

### Option D: Hybrid with Fallback

**Behavior:** Attempt prompt injection, but gracefully degrade if parsing fails.

```rust
fn handle_chat_completion(request: ChatCompletionRequest) -> Response {
    if request.tools.is_some() {
        // Inject tools into prompt
        let modified_request = inject_tools(request);
        let response = call_copilot(modified_request).await?;

        // Try to parse tool calls
        match parse_tool_calls(&response.content) {
            Ok(tool_calls) if !tool_calls.is_empty() => {
                // Return response with tool_calls
                return response_with_tools(response, tool_calls);
            }
            _ => {
                // No tool calls found, return as regular message
                return response;
            }
        }
    }

    // No tools requested, pass through
    call_copilot(request).await
}
```

**Pros:**
- Best effort tool support
- Graceful degradation
- No hard failures

**Cons:**
- Unpredictable behavior
- User may not know if tools were used or ignored

---

## Recommended Approach

**Phase 1 (v0.1):** Option A — No support, clear error message

```json
{
  "error": {
    "message": "Tools/functions are not supported. GitHub Copilot does not provide native function calling.",
    "type": "invalid_request_error",
    "param": "tools"
  }
}
```

**Phase 2 (v0.2+):** Option D — Hybrid with opt-in flag

```bash
copilot-adapter start --experimental-tools
```

Or via request header:
```
X-Copilot-Adapter-Tools: prompt-injection
```

This allows users who want tool support to opt-in while understanding it's experimental.

---

## Technical Implementation Details (For Phase 2)

### Tool Definition Formatting

```rust
// src/tools/formatter.rs

pub fn format_tools_for_prompt(tools: &[Tool]) -> String {
    let mut output = String::from("# Available Functions\n\n");

    for tool in tools {
        output.push_str(&format!(
            "## {}\n{}\n\nParameters:\n```json\n{}\n```\n\n",
            tool.function.name,
            tool.function.description.as_deref().unwrap_or(""),
            serde_json::to_string_pretty(&tool.function.parameters).unwrap()
        ));
    }

    output.push_str(TOOL_USAGE_INSTRUCTIONS);
    output
}

const TOOL_USAGE_INSTRUCTIONS: &str = r#"
# How to Call Functions

When you need to call a function, respond with a JSON block:

```json
{"function_call": {"name": "function_name", "arguments": {"param": "value"}}}
```

You may call multiple functions by including multiple JSON blocks.
After the function results are provided, continue your response.
"#;
```

### Response Parsing

```rust
// src/tools/parser.rs

use regex::Regex;
use serde_json::Value;

pub fn parse_tool_calls(content: &str) -> Vec<ToolCall> {
    let mut calls = Vec::new();

    // Pattern: ```json\n{"function_call": ...}\n```
    let re = Regex::new(r#"```json\s*(\{[^`]+\})\s*```"#).unwrap();

    for cap in re.captures_iter(content) {
        if let Ok(json) = serde_json::from_str::<Value>(&cap[1]) {
            if let Some(fc) = json.get("function_call") {
                if let (Some(name), Some(args)) = (
                    fc.get("name").and_then(|n| n.as_str()),
                    fc.get("arguments")
                ) {
                    calls.push(ToolCall {
                        id: format!("call_{}", uuid::Uuid::new_v4()),
                        r#type: "function".to_string(),
                        function: FunctionCall {
                            name: name.to_string(),
                            arguments: args.to_string(),
                        },
                    });
                }
            }
        }
    }

    calls
}
```

### Configuration

```rust
// src/config.rs

pub struct AdapterConfig {
    pub port: u16,
    pub host: String,
    pub experimental_tools: bool,  // --experimental-tools flag
    pub tool_format: ToolFormat,   // xml, json, markdown
}

pub enum ToolFormat {
    Json,       // Default, best for GPT-4
    Xml,        // LiteLLM-style
    Markdown,   // Human-readable, decent parsing
}
```

---

## Testing Strategy

### Unit Tests

1. Tool definition formatting (all formats)
2. Response parsing (valid calls, no calls, malformed)
3. Edge cases (nested JSON, escaped characters, multiple calls)

### Integration Tests

1. Request with tools → prompt correctly modified
2. Response with tool call → correctly parsed and returned
3. Response without tool call → returned as-is
4. Concurrent requests with/without tools

### Manual E2E Tests

1. Simple single-tool call (get_weather)
2. Multi-tool call (get_weather + search)
3. Tool with complex parameters (nested objects)
4. Conversation with tool results fed back

---

## Open Questions

| # | Question | Current Thinking |
|---|----------|------------------|
| 1 | Which format (XML vs JSON vs Markdown) works best with Copilot's GPT-4? | Need empirical testing; JSON likely best |
| 2 | Should tool_choice be supported? | Start with "auto" only |
| 3 | How to handle parallel_tool_calls? | Not supported initially |
| 4 | Should we strip tool calls from visible content? | Yes, return clean content + tool_calls |
| 5 | Rate limiting impact of longer prompts? | Monitor and document |

---

## References

- [LiteLLM Function Calling Documentation](https://docs.litellm.ai/docs/completion/function_call)
- [LiteLLM Source - factory.py](https://github.com/BerriAI/litellm/blob/main/litellm/litellm_core_utils/prompt_templates/factory.py)
- [OpenAI Function Calling Guide](https://platform.openai.com/docs/guides/function-calling)
- [DESIGN.md](./DESIGN.md) — Main adapter design document
