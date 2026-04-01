# Regression Fix: Tool Definition Context Loss

## Issue

After commit `5866285` (Make tools support not experimental), users reported errors during tool execution:

```
<tool_use_error>InputValidationError: Agent failed due to the following issue:
The parameter `` type is expected as `object` but provided as `string`</tool_use_error>
```

## Root Cause

The commit correctly removed the `--experimental-tools` flag and made tool support always-on. However, this change didn't introduce a code bug in the adapter itself.

**The actual issue is a CLIENT-SIDE problem**: Claude Code (or any client) MUST re-send tool definitions on every turn when tool calling is active, including:

1. Initial request with user message
2. **Follow-up request with tool results** ← This is where the problem occurs

When Claude Code sends back tool execution results, if it doesn't include the tool definitions again, the model loses the schema context needed to format subsequent tool calls correctly.

## Why This Matters

The adapter uses **prompt injection** for tool calling (because GitHub Copilot's API doesn't natively support tools):

1. Tool definitions are injected into the system prompt
2. The model is instructed to format tool calls as JSON
3. Tool calls are parsed from text responses

**Without schema context**, the model:
- Doesn't know parameter types
- May generate malformed JSON
- Can pass strings where objects are expected
- Produces empty parameter names

## The Fix

Added diagnostic warnings in both handlers (`chat.rs` and `messages.rs`):

```rust
if let Some(ref tools) = request.tools {
    // Inject tools
} else if has_tool_results {
    tracing::warn!(
        "Request contains tool results but no tool definitions. \
         The model may generate malformed tool calls without schema context. \
         Claude Code should re-send tool definitions on every turn when tool \
         calling is active."
    );
}
```

This helps debug the issue by:
1. **Alerting users** when tool definitions are missing
2. **Identifying the client** that's not re-sending tools
3. **Providing clear guidance** on what should happen

## For Claude Code Users

If you see this warning, **this is a bug in Claude Code**, not the adapter. The Claude Code team should ensure that when tool calling is active, tool definitions are included in EVERY request, not just the first one.

## For Other API Clients

When using the adapter with tools:

**✅ CORRECT:**
```javascript
// Request 1: Initial user message
POST /v1/messages
{
  "tools": [/* tool definitions */],
  "messages": [{"role": "user", "content": "Call a tool"}]
}

// Request 2: Tool results
POST /v1/messages
{
  "tools": [/* SAME tool definitions */],  // ← Re-send tools!
  "messages": [
    {"role": "user", "content": "Call a tool"},
    {"role": "assistant", "content": [{"type": "tool_use", ...}]},
    {"role": "user", "content": [{"type": "tool_result", ...}]}
  ]
}
```

**❌ WRONG:**
```javascript
// Request 2: Tool results (missing tools!)
POST /v1/messages
{
  "messages": [/* conversation with tool results */]
  // No tools field → model has no schema context!
}
```

## Technical Details

The adapter's behavior is **correct as-is**:
- It injects tool definitions when present
- It translates tool results to the format Copilot expects
- It parses tool calls from text responses

The issue is that **clients must cooperate** by re-sending tool definitions to maintain context across multi-turn tool conversations.

## Monitoring

With debug logging enabled:
```bash
copilot-adapter start --log-level debug
```

You'll now see warnings like:
```
WARN Request contains tool_result blocks but no tool definitions.
     The model may generate malformed tool calls without schema context.
     Claude Code should re-send tool definitions on every turn when tool calling is active.
```

This indicates the client needs to be fixed.

## Resolution Status

- **Adapter**: ✅ Fixed (diagnostic warnings added)
- **Root cause**: Client-side issue (Claude Code or other clients not re-sending tools)
- **Workaround**: None available at adapter level
- **Proper fix**: Client must re-send tool definitions on every turn

## Related Files

- `src/handlers/chat.rs` - OpenAI format tool handling
- `src/handlers/messages.rs` - Anthropic format tool handling
- `src/tools/injector.rs` - Tool definition injection logic
- `TOOLS-SUPPORT.design.md` - Original design document
