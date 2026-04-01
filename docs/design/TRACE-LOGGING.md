# Trace Logging Feature

## Overview

The copilot-adapter now includes comprehensive trace-level logging that captures the full request/response flow at every transformation point. This is invaluable for debugging, understanding the adapter's behavior, and troubleshooting issues with tool calls, model normalization, and format translation.

## How to Enable

```bash
# Start with trace logging enabled
copilot-adapter start --log-level trace

# Or use the RUST_LOG environment variable
RUST_LOG=trace copilot-adapter start

# With daemon mode and log file
copilot-adapter start --daemon --log-level trace --log-file /tmp/adapter-trace.log
```

## What Gets Logged

When trace logging is enabled, the adapter logs the complete JSON payload at four key points in the request/response lifecycle:

### 1. Incoming from Claude Code
- Full request as received from Claude Code
- Includes all fields: model, messages, tools, parameters, etc.
- Format: Anthropic (for `/v1/messages`) — the sole Claude Code entrypoint

### 2. Outgoing to GitHub Copilot API
- Full request after all transformations
- Shows: model name normalization, tool injection, format translation
- Always in OpenAI format (even if original was Anthropic)

### 3. Incoming from GitHub Copilot API
- Full response/chunks received from GitHub
- For streaming: individual SSE chunks logged as received
- Always in OpenAI format

### 4. Outgoing to Claude Code
- Final response after all post-processing
- Shows: tool call parsing, format translation back to Anthropic (if needed)
- Format matches the original request format

## Structured Log Fields

All trace logs include structured fields for easy filtering:

| Field | Values | Description |
|-------|--------|-------------|
| `direction` | `INCOMING`, `OUTGOING` | Whether data is coming in or going out |
| `source` | `Claude Code`, `GitHub Copilot API` | Where the data came from |
| `destination` | `Claude Code`, `GitHub Copilot API` | Where the data is going |
| `endpoint` | `/v1/messages`, `/chat/completions` | API endpoint |
| `format` | `OpenAI`, `Anthropic` | Data format |
| `mode` | `streaming`, `streaming_with_tools`, `non-streaming` | Request mode |
| `chunk_type` | `role`, `text_content`, `tool_calls` | Type of SSE chunk (streaming only) |

## Example Log Output

### Non-streaming request flow:

```
TRACE direction=INCOMING source="Claude Code" endpoint="/v1/messages" request_json="{...}" Full request received from Claude Code (Anthropic format)
TRACE direction=OUTGOING destination="GitHub Copilot API" endpoint="/chat/completions" format="OpenAI (translated from Anthropic)" request_json="{...}" Full request being sent to GitHub Copilot API (Anthropic endpoint)
TRACE direction=INCOMING source="GitHub Copilot API" endpoint="/chat/completions" response_json="{...}" Full response received from GitHub Copilot API (Anthropic endpoint)
TRACE direction=OUTGOING destination="Claude Code" endpoint="/v1/messages" format="Anthropic" response_json="{...}" Final response being sent to Claude Code (Anthropic format)
```

### Streaming request flow:

```
TRACE direction=INCOMING source="Claude Code" endpoint="/v1/messages" request_json="{...}" Full request received from Claude Code (Anthropic format)
TRACE direction=OUTGOING destination="GitHub Copilot API" endpoint="/chat/completions" mode="streaming" Initiating streaming request to GitHub Copilot API (Anthropic endpoint)
TRACE direction=INCOMING source="GitHub Copilot API" format="OpenAI" mode="streaming" chunk_json="{...}" Received SSE chunk from GitHub Copilot API (will translate to Anthropic)
TRACE direction=INCOMING source="GitHub Copilot API" format="OpenAI" mode="streaming" chunk_json="{...}" Received SSE chunk from GitHub Copilot API (will translate to Anthropic)
...
```

### Streaming with tools (buffered):

```
TRACE direction=INCOMING source="Claude Code" endpoint="/v1/messages" request_json="{...}" Full request received from Claude Code
TRACE direction=OUTGOING destination="GitHub Copilot API" endpoint="/chat/completions" mode="streaming" Initiating streaming request to GitHub Copilot API
TRACE direction=INCOMING source="GitHub Copilot API" mode="streaming_with_tools" chunk_json="{...}" Received SSE chunk from GitHub Copilot API (buffering for tool parsing)
TRACE direction=INCOMING source="GitHub Copilot API" mode="streaming_with_tools" chunk_json="{...}" Received SSE chunk from GitHub Copilot API (buffering for tool parsing)
...
TRACE direction=OUTGOING destination="Claude Code" mode="streaming_with_tools" chunk_type="role" chunk_json="{...}" Sending synthetic role chunk to Claude Code
TRACE direction=OUTGOING destination="Claude Code" mode="streaming_with_tools" chunk_type="text_content" chunk_json="{...}" Sending synthetic text content chunk to Claude Code
TRACE direction=OUTGOING destination="Claude Code" mode="streaming_with_tools" chunk_type="tool_calls" chunk_json="{...}" Sending synthetic tool_calls chunk to Claude Code
```

## Use Cases

### 1. Debugging Tool Call Issues
See exactly:
- What tools were injected into the system prompt
- How the model responded
- Whether tool calls were parsed successfully
- The final tool_calls structure sent to Claude Code

### 2. Understanding Model Name Normalization
Track how model names are transformed:
- Original: `claude-haiku-4-5-20251001` (from Claude Code)
- Normalized: `claude-haiku-4.5` (sent to GitHub Copilot)
- Actual: `claude-haiku` (used by Copilot, from response)

### 3. Format Translation Analysis
See the complete transformation between Anthropic and OpenAI formats:
- Anthropic content blocks → OpenAI message content
- OpenAI tool_calls → Anthropic tool_use blocks
- System prompts, message roles, streaming events

### 4. Streaming Diagnostics
Debug streaming issues:
- When chunks are received vs sent
- Buffering behavior for tool parsing
- Synthetic chunk generation for tool calls
- SSE event ordering

### 5. Image/Vision Content Translation
Track how multimodal content is transformed:
- Base64 image blocks → data URIs
- URL image sources (passthrough)
- Document blocks (skipped with warnings)

## Performance Considerations

**⚠️ WARNING:** Trace logging is **very verbose** and includes full JSON payloads for every request/response.

**Impact:**
- Large log files (can grow to hundreds of MB quickly)
- Increased CPU usage (JSON serialization overhead)
- Potential latency increase (logging overhead on every chunk)
- Logs include full message content (sensitive data warning)

**Recommendations:**
- ✅ Use for debugging specific issues
- ✅ Use with `--log-file` to avoid cluttering stderr
- ✅ Rotate/clean log files regularly
- ❌ Do NOT use in production
- ❌ Do NOT enable for long-running sessions
- ❌ Do NOT log to network destinations

## Filtering Trace Logs

Use standard log filtering tools to extract relevant information:

```bash
# Show only incoming requests from Claude Code
grep 'direction=INCOMING source="Claude Code"' adapter.log

# Show only what was sent to GitHub Copilot
grep 'direction=OUTGOING destination="GitHub Copilot API"' adapter.log

# Show only tool-related processing
grep 'tool' adapter.log

# Show streaming chunks
grep 'chunk_json' adapter.log

# Show format translation
grep 'format="OpenAI (translated from Anthropic)"' adapter.log
```

## Code Implementation

The trace logging is implemented in:
- `src/handlers/messages.rs` - Anthropic messages endpoint (the sole Claude Code entrypoint)

Key implementation patterns:
```rust
// Check if trace logging is enabled (minimal overhead when disabled)
if tracing::enabled!(tracing::Level::TRACE) {
    if let Ok(json) = serde_json::to_string_pretty(&request) {
        tracing::trace!(
            direction = "INCOMING",
            source = "Claude Code",
            endpoint = "/v1/messages",
            request_json = %json,
            "Full request received from Claude Code"
        );
    }
}
```

## Conversation Logging

For easier debugging without full trace-level JSON dumps, the adapter can
write human-readable conversation summaries to a file:

```bash
copilot-adapter start --conversation-log /tmp/conversations.log
```

Each request/response cycle produces a single log entry showing four sections:

1. **FROM CLAUDE CODE** — Incoming model, message count, tool names, system prompt size
2. **TO GITHUB COPILOT API** — Outgoing model (after normalization), message count, whether tools were injected, XML injection size
3. **FROM GITHUB COPILOT API** — Response model, finish reason, content preview, tool call presence
4. **TO CLAUDE CODE** — Final stop reason, content block count, parsed tool call details

### Log rotation

When the log file exceeds `--conversation-log-max-size` (default: 10 MB), the
current file is renamed to `<path>.1` and a new file is started.

```bash
# Custom rotation size (5 MB)
copilot-adapter start --conversation-log /tmp/conv.log --conversation-log-max-size 5000000
```

### Example output

```
================================================================================
[2026-03-30 22:15:00.123 UTC] Request #1 (abc-123)
================================================================================

>>> FROM CLAUDE CODE (Anthropic format)
Model: claude-sonnet-4-20250514
Stream: true
Messages: 3
System prompt: 1234 chars
Tools (2): read_file, write_file
  [0] role=user, 45 chars: Can you read the main.rs file?
  [1] role=assistant [tool_use], 120 chars: I'll read that file for you.
  [2] role=user [tool_result], 500 chars: fn main() { ...

--------------------------------------------------------------------------------
>>> TO GITHUB COPILOT API (OpenAI format)
Model: claude-sonnet-4
Messages: 4
Tools injected: true
XML injection size: 2048 bytes

--------------------------------------------------------------------------------
<<< FROM GITHUB COPILOT API (OpenAI format)
Model: claude-sonnet-4
Finish reason: stop
Has tool calls: true
Content preview: Here is the file content. Let me now...

--------------------------------------------------------------------------------
<<< TO CLAUDE CODE (Anthropic format)
Stop reason: tool_use
Content blocks: 2
  [0] type=text: Here is the file content.
  [1] type=tool_use: read_file (id=call_abc123)
Parsed tool calls: 1
  - read_file (id=call_abc123): {"path":"/src/main.rs"}

================================================================================
```

## Debug Tools Mode

For tool-specific debugging without full trace logs or conversation logging:

```bash
copilot-adapter start --debug-tools
```

This emits additional INFO-level logs for:

- **Tool injection:** Number of tools, tool names, XML prompt size, and an XML preview (first 500 chars)
- **Tool parsing:** Number of parsed tool calls, tool names, or diagnostic info when no calls are found (content length, presence of known tags, content preview)

Example log output:

```
INFO DEBUG_TOOLS: Injecting tools into system prompt num_tools=5 tool_names=["read_file", "write_file", "bash", "grep", "glob"] xml_size=4096 xml_preview="<tools>..."
INFO DEBUG_TOOLS: Successfully parsed tool calls num_calls=1 tool_names=[Some("read_file")]
```

When no tool calls are parsed:

```
INFO DEBUG_TOOLS: No tool calls parsed from response content_length=1234 has_invoke=false has_tool_name=false has_function_calls=false content_preview="Here is my response..."
```

### When to use each option

| Need | Use |
|------|-----|
| Quick overview of request/response flow | `--conversation-log` |
| Tool injection/parsing issues | `--debug-tools` |
| Full JSON payloads at every stage | `--log-level trace` |
| Production debugging | `--conversation-log` (lowest overhead) |

## See Also

- `README.md` - Full documentation including debug logging section
- `CLAUDE.md` - Development notes including trace logging details
- `docs/debugging-tool-calls.md` - Specific guidance for debugging tool issues
