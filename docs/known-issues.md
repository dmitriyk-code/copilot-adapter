# Known Issues

## Multiple Responses from Claude Code

### Description
When using Claude Code through the copilot-adapter, you may see two responses
for a single message. The adapter receives two separate API requests from Claude
Code and proxies both faithfully.

### Likely Cause
This is believed to be caused by Claude Code's background session-title
generation. Claude Code sends a secondary request using a fast, cheap model
(e.g., Haiku) to generate a title for the conversation. This title generation
request:
- Uses a different model than your conversation
- Has no conversation history (only sees e.g. "Let's implement that", not what
  "that" refers to)
- Returns a response asking for clarification because it lacks context

> **Note:** This cause is a hypothesis based on observed behavior (different
> model, different request ID, missing history). Other possible causes — such as
> request duplication in the adapter or a race condition — have not been fully
> ruled out. See `ISSUE-DUAL-RESPONSES.md` for the full investigation.

### What You'll See
1. A response from Haiku asking "What would you like me to implement?" (or
   similar clarification)
2. A response from your selected model (e.g., Sonnet) with the actual answer

In some cases, the response from your selected model may also behave
unexpectedly — for example, generating a markdown code block instead of proper
tool calls, which prevents tool execution from succeeding. This is a separate
issue related to tool call formatting.

### Workaround
- Focus on the response from your selected model and ignore the title
  generator's response.
- If your selected model's response also fails (e.g., tools don't execute),
  retry the request. Tool call formatting issues are intermittent.
- Enable trace logging (`--log-level trace`) to inspect the full request and
  response flow if issues persist.

### Status
Under investigation. The adapter correctly proxies all requests it receives,
but the root cause of the dual requests has not been definitively confirmed.
See `ISSUE-DUAL-RESPONSES.md` for the detailed bug report and investigation
notes.

---

## Parameter Type Coercion (XML Mode)

### Description
When using XML-based tool injection (the default), the XML parser historically
converted all parameter values to strings. This caused MCP validation errors
for tools expecting typed parameters (numbers, booleans, etc.).

### Resolution
**Fixed.** The adapter now uses a `ToolRegistry` that inspects tool schemas
from the request to coerce XML parameter values to their expected types.
Numbers, booleans, objects, and arrays are parsed accordingly. Unknown
parameters fall back to strings.

This fix applies to the XML injection path. When using `--native-tools`,
parameter types are preserved automatically by the OpenAI function calling
format.

---

## Buffered Streaming with XML Tools

### Description
When using XML-based tool injection (the default), streaming responses that
contain tool calls are **buffered entirely** before being emitted to the
client. This means text, tool calls, and follow-up content all appear at once
rather than streaming progressively.

### Workaround
Use `--native-tools` mode for progressive streaming:
```bash
copilot-adapter start --native-tools
```

In native tools mode, text and tool calls stream incrementally as they are
generated, matching the native Anthropic API behavior.

### Status
This is by design for the XML path — tool calls cannot be parsed until the
closing `</function_calls>` XML tag arrives. Use native tools mode for a
better streaming UX.
