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
- Format: Anthropic (for `/v1/messages`) or OpenAI (for `/v1/chat/completions`)

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
| `endpoint` | `/v1/messages`, `/v1/chat/completions`, `/chat/completions` | API endpoint |
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
TRACE direction=INCOMING source="Claude Code" endpoint="/v1/chat/completions" request_json="{...}" Full request received from Claude Code
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
- `src/handlers/chat.rs` - OpenAI chat completions endpoint
- `src/handlers/messages.rs` - Anthropic messages endpoint

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

## See Also

- `README.md` - Full documentation including debug logging section
- `CLAUDE.md` - Development notes including trace logging details
- `docs/debugging-tool-calls.md` - Specific guidance for debugging tool issues
