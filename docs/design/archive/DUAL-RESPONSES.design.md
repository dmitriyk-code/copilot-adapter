# Tool Format Migration and Dual Response Issues — Design Document

**Status:** Draft
**Date:** 2026-03-30
**Severity:** High
**Issue Tracking:** `ISSUE-DUAL-RESPONSES.md`

---

## Executive Summary

Two issues were discovered when using Claude Code through the copilot-adapter:

1. **Issue 1: Dual Concurrent Requests** — Claude Code sends two HTTP requests simultaneously (title generation + conversation). This is **expected behavior** and requires only documentation.

2. **Issue 2: Tool Call Format Mismatch** — The model generates incorrect formats for tool calls. The solution is to **migrate from JSON to XML-based prompt injection**, following the proven LiteLLM/Anthropic Cookbook approach.

Additionally, this document proposes **removing the OpenAI `/v1/chat/completions` endpoint** since the adapter is focused on native Claude Code support (Anthropic API), eliminating unnecessary format conversions.

---

## Issue 1: Dual Concurrent Requests (Documentation Only)

### Root Cause

Claude Code intentionally sends two separate HTTP requests:
- **Request 1**: Title generation using Haiku (1 message, no history, no tools)
- **Request 2**: Main conversation using the user's selected model (full history, tools)

This is by design. The adapter correctly handles both requests.

### Solution

Document this behavior in `docs/known-issues.md`. No code changes required.

---

## Issue 2: Tool Call Format Mismatch

### Current State

The adapter uses **JSON-based prompt injection**:

```json
{"function_call": {"name": "function_name", "arguments": {"param": "value"}}}
```

### Problem

The model generates various incorrect formats:
- `<answer>...</answer>` tags
- Markdown code blocks with natural language
- Mixed formats

### Research: LiteLLM's Proven XML Approach

LiteLLM uses an **XML-based format** that comes directly from the [Anthropic Function Calling Cookbook](https://github.com/anthropics/anthropic-cookbook/blob/main/function_calling/function_calling.ipynb).

#### LiteLLM Tool Prompt Injection

```python
def construct_tool_use_system_prompt(tools):
    tool_use_system_prompt = (
        "In this environment you have access to a set of tools you can use to answer the user's question.\n"
        "\n"
        "You may call them like this:\n"
        "<function_calls>\n"
        "<invoke>\n"
        "<tool_name>$TOOL_NAME</tool_name>\n"
        "<parameters>\n"
        "<$PARAMETER_NAME>$PARAMETER_VALUE</$PARAMETER_NAME>\n"
        "...\n"
        "</parameters>\n"
        "</invoke>\n"
        "</function_calls>\n"
        "\n"
        "Here are the tools available:\n"
        "<tools>\n" + tool_descriptions + "\n</tools>"
    )
```

#### LiteLLM Tool Definition Format

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
<description>The city and state</description>
<required>true</required>
</parameter>
</parameters>
</tool_description>
```

#### LiteLLM Response Parsing

```python
if contains_tag("invoke", outputText):
    function_name = extract_between_tags("tool_name", outputText)[0]
    function_arguments_str = extract_between_tags("invoke", outputText)[0].strip()
    function_arguments = parse_xml_params(function_arguments_str, json_schema)
```

### Why XML is Better for Claude Models

1. **Native training** — Claude models are trained on XML-structured tool calling
2. **Anthropic-recommended** — Official cookbook uses this format
3. **Production-proven** — LiteLLM uses this at scale
4. **Consistent** — Claude natively generates `<invoke>`, `<tool_name>`, `<parameters>` patterns

---

## Proposed Architecture Change: Remove OpenAI Endpoint

### Current State

The adapter exposes two API endpoints:
- `POST /v1/chat/completions` — OpenAI format
- `POST /v1/messages` — Anthropic format

The `/v1/messages` (Anthropic) endpoint performs these conversions:
1. Anthropic request → OpenAI request (for internal processing)
2. Inject tools into OpenAI-format messages
3. Send to Copilot API (which accepts OpenAI format)
4. OpenAI response → Anthropic response

The `/v1/chat/completions` (OpenAI) endpoint:
1. Accepts OpenAI-format request
2. Inject tools into messages
3. Send to Copilot API
4. Return OpenAI-format response

### Problem

- **Unnecessary complexity** — The `/v1/messages` endpoint converts to OpenAI internally, then back
- **Claude Code uses Anthropic API** — Claude Code sends Anthropic-format requests
- **Dual maintenance** — Both endpoints need tool injection and parsing
- **Format confusion** — Internal OpenAI types used even for Anthropic endpoint

### Proposed Change: Remove OpenAI Endpoint

Since Claude Code uses the Anthropic API format, remove the OpenAI endpoint:

1. **Delete** `src/handlers/chat.rs` — OpenAI endpoint handler
2. **Remove** `/v1/chat/completions` route from `src/server.rs`
3. **Simplify** `src/handlers/messages.rs` — No longer needs to convert Anthropic→OpenAI→Anthropic
4. **Streamline** tool injection — Work directly with Anthropic message format
5. **Clean up** `src/copilot/types.rs` — Remove OpenAI-specific types not needed by Copilot client

### Benefits

- **Simpler code** — One endpoint, one format
- **No format conversions** — Request arrives in Anthropic format, response returns in Anthropic format
- **Clearer tool injection** — Inject directly into Anthropic system prompt
- **Less code to maintain** — Fewer handlers, fewer type conversions

### What Remains in OpenAI Format

The **Copilot API** uses OpenAI-compatible format (chat completions), so:
- `src/copilot/client.rs` — Still sends OpenAI-format requests to Copilot
- `src/copilot/types.rs` — Keeps OpenAI types needed for Copilot API communication

The flow becomes:
```
Claude Code (Anthropic) → Adapter → Internal Translation → Copilot API (OpenAI)
                                            ↓
Claude Code (Anthropic) ← Adapter ← Internal Translation ← Copilot API (OpenAI)
```

This is unavoidable because Copilot speaks OpenAI format.

---

## Recommended Solution: XML Format + Remove OpenAI Endpoint

### Phase 1: Switch to XML Tool Format

Replace JSON prompt injection with LiteLLM-style XML:

**File: `src/tools/injector.rs`**

```rust
pub const TOOL_USAGE_INSTRUCTIONS: &str = r#"
In this environment you have access to a set of tools you can use to answer the user's question.

You may call them like this:
<function_calls>
<invoke>
<tool_name>$TOOL_NAME</tool_name>
<parameters>
<$PARAMETER_NAME>$PARAMETER_VALUE</$PARAMETER_NAME>
...
</parameters>
</invoke>
</function_calls>

You can call multiple tools by including multiple <invoke> blocks inside <function_calls>.
"#;

pub fn format_tools_as_xml(tools: &[Tool]) -> String {
    let mut xml = String::from("<tools>\n");
    for tool in tools {
        xml.push_str("<tool_description>\n");
        xml.push_str(&format!("<tool_name>{}</tool_name>\n", tool.function.name));
        if let Some(desc) = &tool.function.description {
            xml.push_str(&format!("<description>\n{}\n</description>\n", desc));
        }
        if let Some(params) = &tool.function.parameters {
            xml.push_str("<parameters>\n");
            xml.push_str(&format_parameters_xml(params));
            xml.push_str("</parameters>\n");
        }
        xml.push_str("</tool_description>\n");
    }
    xml.push_str("</tools>");
    xml
}
```

**File: `src/tools/parser.rs`**

Replace JSON parsing with XML parsing:

```rust
/// Parse tool calls from model-generated text content.
///
/// Looks for `<function_calls>` blocks containing `<invoke>` elements:
/// ```xml
/// <function_calls>
/// <invoke>
/// <tool_name>Read</tool_name>
/// <parameters>
/// <file_path>/src/main.rs</file_path>
/// </parameters>
/// </invoke>
/// </function_calls>
/// ```
pub fn parse_tool_calls(content: &str) -> Vec<ToolCall> {
    // Primary: Look for <function_calls> wrapper
    let wrapped_calls = parse_wrapped_function_calls(content);
    if !wrapped_calls.is_empty() {
        return wrapped_calls;
    }

    // Fallback: Look for standalone <invoke> blocks
    parse_standalone_invokes(content)
}
```

### Phase 2: Remove OpenAI Endpoint

1. Delete `src/handlers/chat.rs`
2. Remove route from `src/server.rs`
3. Simplify `src/handlers/messages.rs` — keep Anthropic→OpenAI conversion (needed for Copilot API)
4. Clean up unused types

### Phase 3: Simplify Internal Types

After removing the OpenAI endpoint, the internal type flow is:

```
AnthropicRequest → (translate) → CopilotRequest → Copilot API
CopilotResponse → (translate) → AnthropicResponse
```

Consider renaming `src/copilot/types.rs` types to clarify they're for Copilot API communication, not for the external API.

---

## Detailed Technical Changes

### Files to Delete

| File | Reason |
|------|--------|
| `src/handlers/chat.rs` | OpenAI endpoint handler no longer needed |

### Files to Modify

| File | Changes |
|------|---------|
| `src/server.rs` | Remove `/v1/chat/completions` route |
| `src/handlers/mod.rs` | Remove `chat` module export |
| `src/tools/injector.rs` | Replace JSON format with XML format |
| `src/tools/parser.rs` | Replace JSON parsing with XML parsing |
| `src/handlers/messages.rs` | Simplify (already uses internal conversion, no major changes) |

### Files Unchanged

| File | Reason |
|------|--------|
| `src/copilot/client.rs` | Still needed to communicate with Copilot API |
| `src/copilot/types.rs` | Still needed for Copilot API types |
| `src/anthropic/types.rs` | Still needed for Anthropic API types |

---

## XML Tool Format Specification

### Tool Definition Format

```xml
<tool_description>
<tool_name>Edit</tool_name>
<description>
Performs exact string replacements in files.
</description>
<parameters>
<parameter>
<name>file_path</name>
<type>string</type>
<description>The absolute path to the file to modify</description>
<required>true</required>
</parameter>
<parameter>
<name>old_string</name>
<type>string</type>
<description>The text to replace</description>
<required>true</required>
</parameter>
<parameter>
<name>new_string</name>
<type>string</type>
<description>The text to replace it with</description>
<required>true</required>
</parameter>
</parameters>
</tool_description>
```

### Tool Invocation Format

```xml
<function_calls>
<invoke>
<tool_name>Edit</tool_name>
<parameters>
<file_path>/src/main.rs</file_path>
<old_string>fn main()</old_string>
<new_string>fn main() -> Result<()></new_string>
</parameters>
</invoke>
</function_calls>
```

### Multiple Tool Calls

```xml
<function_calls>
<invoke>
<tool_name>Read</tool_name>
<parameters>
<file_path>/src/lib.rs</file_path>
</parameters>
</invoke>
<invoke>
<tool_name>Grep</tool_name>
<parameters>
<pattern>TODO</pattern>
<path>/src</path>
</parameters>
</invoke>
</function_calls>
```

---

## Parser Implementation Details

### XML Extraction Functions

Following LiteLLM's approach:

```rust
/// Extract content between XML tags using regex.
fn extract_between_tags(tag: &str, content: &str) -> Vec<String> {
    let pattern = format!(r"<{tag}>(.+?)</{tag}>");
    let regex = Regex::new(&pattern).unwrap();
    regex.captures_iter(content)
        .filter_map(|cap| cap.get(1).map(|m| m.as_str().to_string()))
        .collect()
}

/// Check if a tag exists in the content.
fn contains_tag(tag: &str, content: &str) -> bool {
    let pattern = format!(r"<{tag}>(.+?)</{tag}>");
    Regex::new(&pattern).unwrap().is_match(content)
}
```

### Parameter Parsing

```rust
/// Parse XML parameters into a JSON object.
fn parse_xml_params(invoke_body: &str) -> serde_json::Value {
    let mut params = serde_json::Map::new();

    // Extract parameters block
    if let Some(params_content) = extract_between_tags("parameters", invoke_body).first() {
        // Parse each parameter as <param_name>value</param_name>
        let param_regex = Regex::new(r"<([^>]+)>([^<]*)</\1>").unwrap();
        for cap in param_regex.captures_iter(params_content) {
            let name = cap.get(1).unwrap().as_str();
            let value = cap.get(2).unwrap().as_str().trim();
            params.insert(name.to_string(), serde_json::Value::String(value.to_string()));
        }
    }

    serde_json::Value::Object(params)
}
```

---

## Backward Compatibility

### Deprecation Strategy

The OpenAI endpoint (`/v1/chat/completions`) will be removed in this change. This is acceptable because:

1. **Target audience** — The adapter is designed for Claude Code, which uses Anthropic API
2. **No known OpenAI endpoint users** — Claude Code always uses `/v1/messages`
3. **Simplification benefit** — Outweighs maintaining unused code

### JSON Tool Format Deprecation

The existing JSON tool format will be removed entirely (not kept as fallback):

1. **Clean break** — Avoids confusion about which format to use
2. **XML proven** — LiteLLM has production data showing XML works well
3. **Simpler parser** — One format to parse, not two

---

## Testing Strategy

### Unit Tests

1. **XML Injector Tests**
   - Tool definitions formatted correctly
   - Multiple tools formatted
   - Parameters with various types
   - Special characters escaped

2. **XML Parser Tests**
   - Single tool call parsed
   - Multiple tool calls parsed
   - Standalone `<invoke>` blocks
   - Nested parameters
   - Missing `<function_calls>` wrapper (fallback)
   - Malformed XML (graceful degradation)

3. **Strip Tool Calls Tests**
   - XML blocks removed
   - Surrounding text preserved
   - Multiple blocks stripped

### Integration Tests

1. **End-to-end with mock Copilot**
   - Request with tools → XML injected → Response parsed
   - Streaming with tools
   - Multiple tool calls in response

### Manual E2E Tests

1. **Claude Code scenarios**
   - File editing (Edit tool)
   - File reading (Read tool)
   - Search (Grep tool)
   - Command execution (Bash tool)

---

## Documentation Updates

### Files to Update

| File | Changes |
|------|---------|
| `CLAUDE.md` | Remove `/v1/chat/completions` mention, update tool format description |
| `README.md` | Update API endpoints section, add known issues link |
| `docs/e2e-testing.md` | Update test procedures for XML format |
| `docs/known-issues.md` | **New file** documenting dual-request behavior |

### CLAUDE.md Updates

```markdown
## API Endpoints

- `GET /health` - Health check
- `POST /v1/messages` - Anthropic-format messages (Claude Code native format)
- `GET /v1/models` - List available models
- `GET /v1/models/:model` - Get model details

**Note:** The `/v1/chat/completions` OpenAI endpoint has been removed.
The adapter focuses on native Claude Code support via the Anthropic API.

## Tool Support

Tool/function support uses XML-based prompt injection following the
Anthropic Cookbook format. Tools are injected as XML definitions and
parsed from XML `<function_calls>` blocks in model responses.
```

---

## Risk Assessment

| Risk | Impact | Probability | Mitigation |
|------|--------|-------------|------------|
| XML format not recognized by model | High | Low | LiteLLM has proven this works; extensive testing |
| Breaking existing users of OpenAI endpoint | Medium | Very Low | Claude Code uses Anthropic endpoint only |
| XML parsing edge cases | Medium | Medium | Comprehensive unit tests; fallback for standalone `<invoke>` |
| Regression in tool calling | High | Low | Full integration test suite; manual E2E testing |

---

## Success Criteria

1. **Tool calls work reliably** — 95%+ of tool call attempts are parsed correctly
2. **No format confusion** — Model consistently generates XML format
3. **Cleaner codebase** — Fewer endpoints, simpler flow
4. **Documentation complete** — Known issues documented, CLAUDE.md updated
5. **Debuggable** — Clear trace logs and optional conversation logging

---

## Debuggability Requirements

### Existing Trace Logging

The adapter already has comprehensive trace logging (see `docs/design/TRACE-LOGGING.md`):
- Full JSON at 4 transformation points
- Structured fields for filtering
- Enabled via `--log-level trace`

### New Requirement: Conversation Log File

Add an optional `--conversation-log <path>` flag that writes a **human-readable summary** of each request/response cycle to a dedicated file. This provides:

1. **Quick debugging** — See the essence without wading through verbose JSON
2. **Audit trail** — Record of all conversations through the adapter
3. **Format comparison** — Easy to see what changed between transformations

### Conversation Log Format

```
================================================================================
[2026-03-30T19:14:46.265Z] Request #42
================================================================================

>>> FROM CLAUDE CODE (Anthropic format)
Model: claude-sonnet-4.5
Stream: true
Messages: 5
System: [2847 chars] "You are Claude Code, Anthropic's official CLI..."
Tools: 47 tools [Edit, Read, Write, Bash, Grep, ...]

Message[0] role=user:
  [text] "<system-reminder>The following skills are available..."
  [text] "Let's carefully implement that"

Message[1] role=assistant:
  [text] "Looking at the code, I can see..."
  [tool_use] id=call_abc123 name=Read {"file_path": "/src/main.rs"}

Message[2] role=user:
  [tool_result] id=call_abc123 "fn main() { ... }"

...

--------------------------------------------------------------------------------

>>> TO GITHUB COPILOT API (OpenAI format)
Model: claude-sonnet-4.5 (normalized from claude-sonnet-4.5)
Messages: 6 (including injected system prompt)

[system] "# Available Functions\n<tools>..."
[user] "Let's carefully implement that"
...

Tool injection: 47 tools injected as XML into system prompt

--------------------------------------------------------------------------------

<<< FROM GITHUB COPILOT API (OpenAI format)
Model: claude-sonnet-4.5
Finish reason: stop
Content: [671 chars]

"I'll implement comprehensive error logging for tool call parsing failures..."

<function_calls>
<invoke>
<tool_name>Edit</tool_name>
<parameters>
<file_path>/src/tools/parser.rs</file_path>
...
</parameters>
</invoke>
</function_calls>

--------------------------------------------------------------------------------

<<< TO CLAUDE CODE (Anthropic format)
Model: claude-sonnet-4.5
Stop reason: tool_use
Content blocks: 2

[text] "I'll implement comprehensive error logging..."
[tool_use] id=call_def456 name=Edit input={"file_path": "/src/tools/parser.rs", ...}

Tool parsing: 1 tool call extracted from XML

================================================================================
```

### Implementation Approach

1. **New CLI flag**: `--conversation-log <path>`
2. **Log writer module**: `src/conversation_log.rs`
3. **Summary extraction**: Functions to extract human-readable summaries from types
4. **Non-blocking writes**: Use async file I/O to avoid latency impact
5. **Rotation**: Optional `--conversation-log-max-size` for log rotation

### Trace Logging Updates for XML

Update trace logs to include XML-specific information:

```rust
// When injecting tools
tracing::debug!(
    num_tools = tools.len(),
    tool_names = ?tools.iter().map(|t| &t.function.name).collect::<Vec<_>>(),
    xml_size = xml_prompt.len(),
    "Injecting tools as XML into system prompt"
);

// When parsing tool calls
tracing::debug!(
    num_calls = calls.len(),
    tool_names = ?calls.iter().map(|tc| &tc.function.name).collect::<Vec<_>>(),
    format = "XML",
    had_wrapper = had_function_calls_wrapper,
    "Parsed tool calls from XML"
);

// When tool call parsing fails
tracing::warn!(
    content_preview = %content.chars().take(500).collect::<String>(),
    contains_invoke = contains_tag("invoke", content),
    contains_tool_name = contains_tag("tool_name", content),
    contains_function_calls = contains_tag("function_calls", content),
    "Content contains tool-like patterns but parsing failed"
);
```

### Debug Mode Flag

Add `--debug-tools` flag that enables verbose tool-related logging at INFO level:

```bash
copilot-adapter start --debug-tools
```

This logs (at INFO, not TRACE):
- Tool definitions being injected
- XML injection preview (first 500 chars)
- Tool calls found in response
- Tool call parsing success/failure with reasons

---

## References

- `ISSUE-DUAL-RESPONSES.md` — Original issue report
- `logs.txt` — Trace logs from the incident
- [LiteLLM Source](https://github.com/BerriAI/litellm) — `litellm/litellm_core_utils/prompt_templates/factory.py`
- [Anthropic Function Calling Cookbook](https://github.com/anthropics/anthropic-cookbook/blob/main/function_calling/function_calling.ipynb)
- `TOOLS-SUPPORT.design.md` — Original JSON-based tools implementation
