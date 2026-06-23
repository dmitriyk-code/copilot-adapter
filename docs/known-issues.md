# Known Issues

## Claude Opus 4.7 Variants and Context Windows (Obsolete)

### Description
GitHub Copilot **previously** exposed Claude Opus 4.7 as four distinct model
SKUs, each with effort level and context window encoded in the model name
(`claude-opus-4.7`, `-high`, `-xhigh`, `-1m-internal`) rather than as separate
`reasoning.effort` API fields. The adapter routed to these SKUs via a special
case in `apply_model_modifiers()`.

### Resolution / current state
**Obsolete — Copilot consolidated these SKUs.** The live
`GET https://api.githubcopilot.com/models` response now exposes only the base
name `claude-opus-4.7` (no `-high` / `-xhigh` / `-1m-internal` variants). The
adapter no longer encodes effort or context in the model name: it sends the
plain base model name and carries effort via the standard `reasoning.effort`
field, the same as every other model. The `apply_model_modifiers()` function has
been removed. See `docs/design/COPILOT-1M-MODEL-CONSOLIDATION.design.md`.

The empirically-measured per-variant context windows documented here previously
no longer apply — there is a single `claude-opus-4.7` model. (Per Anthropic's
model docs, Opus 4.7 is 1M-native.)

---

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

---

## Token Counting: Image Blocks Inside ToolResult

### Description
When counting tokens, top-level `Image` and `Document` blocks use a fixed
estimate of 85 tokens (approximating a low-resolution image tile). However,
`Image` blocks nested inside a `ToolResult`'s `Blocks` content are serialized
to JSON instead, which counts the full serialized content including base64
data. This means a `ToolResult` carrying a large base64 image could produce
a significantly higher token count than the same image at the top level.

### Impact
Token counts for `ToolResult` blocks containing images may be inflated
compared to the same image referenced as a top-level content block. This
primarily affects accuracy of token estimates, not correctness of the API.

### Workaround
No workaround needed — token counts are estimates by design (see NFR1 in the
design document). The inconsistency only affects edge cases where images are
returned inside tool results.

### Status
Known limitation. A follow-up may unify the handling so that `Image` blocks
inside `ToolResult` also use the fixed 85-token estimate, matching top-level
behavior.

---

## Windows Stop Uses Hard Kill

### Description
On Windows, `copilot-adapter stop` uses `taskkill /F` (force kill) to terminate
the adapter process. Unlike Unix SIGTERM, this bypasses the server's graceful
shutdown handler, which normally drains in-flight requests before exiting.

### Impact
In-flight SSE streaming responses may be abruptly truncated when stopping on
Windows. The adapter's status file is cleaned up externally by the stop command,
so no stale state is left behind.

### Workaround
No workaround needed for most use cases. The status file cleanup is handled
externally. If graceful shutdown is critical, stop the adapter when no requests
are in flight, or use Ctrl+C in foreground mode (which triggers a clean
shutdown via the tokio signal handler).

### Status
Known limitation. A future improvement could use a Windows named event or pipe
to signal the adapter process for a graceful shutdown, similar to how Unix
SIGTERM works.

---

## Prompt-Too-Long Errors Returned as 502 (Resolved)

### Description
When the Copilot API returned HTTP 400 with `model_max_prompt_tokens_exceeded`
(e.g., prompt exceeded the ~168K token limit), the adapter wrapped it as a
generic 502 `upstream_error`. Claude Code treated this as an opaque connection
failure rather than a "prompt too long" error, preventing automatic context
compaction.

### Resolution
**Fixed.** The adapter now detects `model_max_prompt_tokens_exceeded` in the
Copilot API response and translates it into an Anthropic-format HTTP 400
`invalid_request_error` with `"code": "prompt_too_long"`. The error message
format (`"prompt is too long: N tokens > M maximum"`) matches Claude Code's
regex, enabling automatic context compaction.

Implementation: `AppError::PromptTooLong` in `src/error.rs`,
`parse_prompt_too_long()` in `src/copilot/client.rs`.

---

## Truncated Tool Calls Silently Dropped (Resolved)

### Description
When the Copilot API returned `finish_reason: "length"` mid-tool-call, the
adapter dropped the incomplete `tool_use` block entirely. Claude Code received
`stop_reason: "max_tokens"` with zero content blocks. Since Claude Code's
internal stream processing had already detected tool_use activity,
`needsFollowUp = true`, which skipped the max_tokens escalation logic
(8K → 64K). The model never got a larger output budget to complete the
tool call.

### Resolution
**Fixed.** The adapter now emits a descriptive text content block when a tool
call is truncated: `[Tool call to "X" was truncated due to output token limit]`.
This gives Claude Code a text-only response (no tool_use blocks), allowing it
to fire the max_tokens escalation logic and retry with a larger output budget.

Implementation: `src/streaming/state.rs` — truncation notice emission in the
streaming state machine.

---

## 1M Context Model Handling

### Description
When Claude Code's user selects "Opus (1M context)" or similar, Claude Code
communicates this via the `anthropic-beta: context-1m-*` HTTP header, not via
the model name (it strips `[1m]` before sending).

### History
An earlier adapter version detected this header and **appended `-1m`** to the
normalized Copilot model name, because Copilot then exposed 1M context as a
separate model ID (e.g., `claude-opus-4.6-1m`).

### Current behavior
GitHub Copilot consolidated its Claude SKUs — the live `/models` list no longer
contains any `-1m` IDs, and the base models are 1M-native (Opus 4.6/4.7/4.8 and
Sonnet 4.6 per Anthropic's model docs; 1M is a context-size toggle per the
Copilot docs). Appending `-1m` would now select a non-existent model. The
adapter therefore **detects the `context-1m` header for diagnostic logging only
and no longer modifies the model name** — the normalized base name is forwarded
unchanged. Implementation: `has_1m_context_beta()` in
`src/handlers/messages.rs`. See
`docs/design/COPILOT-1M-MODEL-CONSOLIDATION.design.md`.

---

## Effort and Thinking Parameters Silently Dropped (Resolved)

### Description
Claude Code sends `output_config.effort` and `thinking` configuration to
control model reasoning behavior. The adapter's `AnthropicRequest` struct had
no fields for these parameters, so they were silently discarded during
deserialization. Additionally, `thinking` and `redacted_thinking` content
blocks in conversation history caused deserialization failures.

### Resolution
**Fixed.** The adapter now accepts `output_config.effort` and translates it to
`reasoning.effort` in the OpenAI request format (`"max"` maps to `"high"`).
`Thinking` and `RedactedThinking` content block variants are accepted during
deserialization and stripped before translation. Temperature is suppressed when
thinking is active to avoid API errors.

Implementation: `OutputConfig` and `strip_thinking_blocks()` in
`src/anthropic/types.rs`, `Reasoning` struct in `src/copilot/types.rs`.

---

## Streaming Token Counts Always Zero (Resolved)

### Description
Claude Code's `/model` view always showed `0/Nm tokens (0%)` for streaming
responses. The GitHub Copilot API does not return `usage` data in its SSE
chunks, and the adapter emitted `input_tokens: 0` / `output_tokens: 0` in
`message_start` and `message_delta` events.

### Resolution
**Fixed.** The adapter now estimates token counts locally using `tiktoken-rs`
(`cl100k_base` encoding). `count_tokens_for_request()` counts input tokens
from the full `AnthropicRequest` before the stream starts, and
`count_output_tokens()` counts accumulated output text and tool-call JSON at
stream finalization. If the upstream Copilot API ever starts returning real
usage data in `ChatCompletionChunk.usage`, those values automatically override
the local estimates.

Implementation: `count_tokens_for_request()` and `count_output_tokens()` in
`src/token_counter.rs`, accumulation fields (`output_text`, `output_tool_json`,
`upstream_input_tokens`, `upstream_output_tokens`) in `src/streaming/state.rs`.
