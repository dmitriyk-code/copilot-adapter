# Tools/Functions Support for Copilot Adapter — Design Document (Draft)

**Status:** Draft / Research
**Date:** 2026-03-26 (updated 2026-03-27)
**Prerequisite:** Core adapter implementation (IMPLEMENTATION.plan.md) — **COMPLETE**

---

## Context

The Copilot Adapter is now fully implemented with:
- **OpenAI-compatible API** (`POST /v1/chat/completions`, `GET /v1/models`)
- **Anthropic-compatible API** (`POST /v1/messages`) with bidirectional format translation

Both API endpoints currently reject `tools`/`functions` parameters (OpenAI) and `tools` parameter (Anthropic) because GitHub Copilot's upstream API does not natively support function calling. This document explores options for adding tool support via prompt injection, similar to how LiteLLM handles non-native function calling.

---

## Problem Statement

GitHub Copilot's API does not natively support tool/function calling:
- **OpenAI format:** `tools` and `functions` parameters are not supported
- **Anthropic format:** `tools` parameter is not supported

When a client sends a request with these parameters, the adapter currently returns a 400 error:
```json
{
  "error": {
    "message": "Tools/functions are not supported. GitHub Copilot does not provide native function calling.",
    "type": "invalid_request_error",
    "param": "tools"
  }
}
```

Users who rely on tool calling for agentic workflows (including Claude Code's native tool use) would be unable to use those workflows through the adapter.

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

### Option A: No Support (Current Implementation)

**Behavior:** Return 400 error when `tools` or `functions` present in request.

This is the **current behavior** in the implemented adapter.

**Pros:**
- Simple, no additional complexity
- No false expectations about reliability
- No parsing edge cases

**Cons:**
- Users with tool-dependent workflows cannot use the adapter
- Feature gap compared to LiteLLM and similar proxies
- Claude Code's native tool use is blocked

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

**Current State (v0.1):** Option A — No support, clear error message

The adapter currently rejects tool parameters with:
```json
{
  "error": {
    "message": "Tools/functions are not supported. GitHub Copilot does not provide native function calling.",
    "type": "invalid_request_error",
    "param": "tools"
  }
}
```

**Future (v0.2+):** Option D — Hybrid with opt-in flag

```bash
copilot-adapter start --experimental-tools
```

Or via request header:
```
X-Copilot-Adapter-Tools: prompt-injection
```

This allows users who want tool support to opt-in while understanding it's experimental.

### Implementation Considerations for Both API Formats

Since the adapter now supports both OpenAI (`/v1/chat/completions`) and Anthropic (`/v1/messages`) formats, tool injection must handle both:

**OpenAI Format:**
- Tools defined in `tools` array with JSON Schema parameters
- Tool calls returned in `choices[0].message.tool_calls`
- Tool results sent as messages with `role: "tool"`

**Anthropic Format:**
- Tools defined in `tools` array with `input_schema`
- Tool calls returned as `tool_use` content blocks
- Tool results sent as `tool_result` content blocks

The translation layer in `src/anthropic/types.rs` would need to be extended to handle tool definitions and tool call/result content blocks.

---

## Technical Implementation Details (For Phase 2)

### Architecture Overview

The tool injection feature would integrate with the existing adapter architecture:

```
Request with tools
        │
        ▼
┌───────────────────┐
│ /v1/chat/completions │  ←── OpenAI format tools
│ /v1/messages         │  ←── Anthropic format tools
└───────────────────┘
        │
        ▼ (if --experimental-tools enabled)
┌───────────────────┐
│ Tool Injector     │  ←── Inject tools into system prompt
└───────────────────┘
        │
        ▼
┌───────────────────┐
│ Copilot Client    │  ←── Send modified request (no tools param)
└───────────────────┘
        │
        ▼
┌───────────────────┐
│ Response Parser   │  ←── Extract tool calls from text
└───────────────────┘
        │
        ▼
┌───────────────────┐
│ Format Translator │  ←── Convert to OpenAI/Anthropic tool_calls
└───────────────────┘
```

### New Files to Create

| File | Purpose |
|------|---------|
| `src/tools/mod.rs` | Tools module exports |
| `src/tools/injector.rs` | Prompt injection logic |
| `src/tools/parser.rs` | Response parsing for tool calls |
| `src/tools/types.rs` | Internal tool representation |

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
// src/cli.rs (extend existing CLI)

#[derive(Parser)]
pub struct StartArgs {
    #[arg(long, default_value = "127.0.0.1")]
    pub host: String,

    #[arg(short, long, default_value = "6767")]
    pub port: u16,

    #[arg(long)]
    pub daemon: bool,

    #[arg(long)]
    pub log_file: Option<PathBuf>,

    #[arg(long, default_value = "info")]
    pub log_level: String,

    // New flag for experimental tools support
    #[arg(long, help = "Enable experimental tool/function support via prompt injection")]
    pub experimental_tools: bool,

    #[arg(long, default_value = "json", help = "Tool prompt format: json, xml, markdown")]
    pub tool_format: ToolFormat,
}

#[derive(Clone, Copy, Default, ValueEnum)]
pub enum ToolFormat {
    #[default]
    Json,       // Best for GPT-4
    Xml,        // LiteLLM-style
    Markdown,   // Human-readable
}
```

### Integration with Existing Handlers

The existing handlers would need minimal changes:

```rust
// src/handlers/chat.rs (extend existing)

pub async fn chat_completions(
    State(state): State<Arc<AppState>>,
    Json(mut request): Json<ChatCompletionRequest>,
) -> Result<Response, AppError> {
    // Check if tools are requested
    if request.tools.is_some() || request.functions.is_some() {
        if !state.config.experimental_tools {
            return Err(AppError::InvalidRequest(
                "Tools/functions are not supported. Enable with --experimental-tools".into()
            ));
        }
        // Inject tools into prompt, remove from request
        request = inject_tools_into_request(request, state.config.tool_format);
    }

    // ... rest of existing handler
}

// src/handlers/messages.rs (extend existing)

pub async fn messages(
    State(state): State<Arc<AppState>>,
    Json(mut request): Json<AnthropicRequest>,
) -> Result<Response, AppError> {
    // Similar tool injection for Anthropic format
    if request.tools.is_some() {
        if !state.config.experimental_tools {
            return Err(AppError::InvalidRequest(
                "Tools are not supported. Enable with --experimental-tools".into()
            ));
        }
        // Inject tools, convert to OpenAI format, then proceed
    }

    // ... rest of existing handler (translate to OpenAI, call Copilot, translate back)
}
```

---

## Testing Strategy

### Unit Tests

1. Tool definition formatting (all formats: JSON, XML, Markdown)
2. Response parsing (valid calls, no calls, malformed)
3. Edge cases (nested JSON, escaped characters, multiple calls)
4. Format translation (OpenAI tools ↔ internal representation)
5. Anthropic tool format handling

### Integration Tests

1. OpenAI request with tools → prompt correctly modified
2. Anthropic request with tools → prompt correctly modified
3. Response with tool call → correctly parsed and returned (both formats)
4. Response without tool call → returned as-is
5. Concurrent requests with/without tools
6. Streaming with tool calls

### Manual E2E Tests

1. Simple single-tool call (get_weather) via `/v1/chat/completions`
2. Simple single-tool call via `/v1/messages`
3. Multi-tool call (get_weather + search)
4. Tool with complex parameters (nested objects)
5. Conversation with tool results fed back
6. Claude Code integration test with native tool use

---

## Open Questions

| # | Question | Current Thinking |
|---|----------|------------------|
| 1 | Which format (XML vs JSON vs Markdown) works best with Copilot's GPT-4? | Need empirical testing; JSON likely best |
| 2 | Should tool_choice be supported? | Start with "auto" only |
| 3 | How to handle parallel_tool_calls? | Not supported initially |
| 4 | Should we strip tool calls from visible content? | Yes, return clean content + tool_calls |
| 5 | Rate limiting impact of longer prompts? | Monitor and document |
| 6 | How to handle Anthropic's `tool_result` content blocks? | Translate to OpenAI `tool` role messages |
| 7 | Should tool injection work with streaming? | Yes, but parsing is more complex |
| 8 | How to handle tool calls that span multiple chunks in streaming? | Buffer until complete JSON found |

---

## References

- [LiteLLM Function Calling Documentation](https://docs.litellm.ai/docs/completion/function_call)
- [LiteLLM Source - factory.py](https://github.com/BerriAI/litellm/blob/main/litellm/litellm_core_utils/prompt_templates/factory.py)
- [OpenAI Function Calling Guide](https://platform.openai.com/docs/guides/function-calling)
- [Anthropic Tool Use Guide](https://docs.anthropic.com/en/docs/tool-use)
- [DESIGN.md](./DESIGN.md) — Main adapter design document
- [IMPLEMENTATION.plan.md](./IMPLEMENTATION.plan.md) — Implementation plan (completed)
- [README.md](./README.md) — User documentation
