# Context Window Enforcement & Truncated Tool Recovery — Design Document

**Status:** Draft
**Date:** 2026-04-05
**Severity:** High
**Related:** `LARGE-FILE-WRITE-BUG-RESEARCH.md`, `ERROR_INVESTIGATION_REPORT.md`

---

## Executive Summary

The copilot-adapter has two related bugs that cause Claude Code sessions to fail during long conversations or large file writes:

1. **Context window mismatch** — Claude Code believes `claude-opus-4-6` has a 200K context window (its built-in default), but the GitHub Copilot API enforces a 168K prompt-token limit. When the prompt exceeds this, the Copilot API returns HTTP 400 `model_max_prompt_tokens_exceeded`. The adapter wraps this as a generic 502 `upstream_error`, which Claude Code treats as an opaque connection failure rather than a "prompt too long" error that would trigger automatic context compaction.

2. **Truncated tool calls silently dropped** — When the Copilot API returns `finish_reason: "length"` mid-tool-call, the adapter drops the incomplete tool_use block entirely. Claude Code receives `stop_reason: "max_tokens"` with zero tool_use content blocks. However, Claude Code's internal stream processing has already detected tool_use activity, so `needsFollowUp = true`, which skips the max_tokens escalation logic (8K → 64K). The model never gets a larger output budget to complete the tool call.

This document designs two targeted fixes:
- **Option A**: Translate Copilot 400 `model_max_prompt_tokens_exceeded` into an Anthropic-format `invalid_request_error` with a message that matches Claude Code's prompt-too-long regex, returning HTTP 400.
- **Option E**: When a tool call is truncated, emit a text content block explaining the truncation instead of dropping it silently, so Claude Code sees a text-only response and can fire max_tokens escalation.

---

## Context / Background

### Current State

#### Error handling for Copilot API 400 responses

**`src/copilot/client.rs:handle_error_response()` (lines 93-112):**
```rust
async fn handle_error_response(response: reqwest::Response) -> AppError {
    let status = response.status();
    if status.as_u16() == 429 {
        // ... rate limit handling ...
        return AppError::RateLimited(retry_after);
    }
    let body = response.text().await.unwrap_or_default();
    AppError::CopilotError(format!("Copilot API returned HTTP {status}: {body}"))
}
```

All non-429 errors become `AppError::CopilotError`, which maps to HTTP 502 + `"type": "upstream_error"` in `src/error.rs`. There is no special handling for HTTP 400 or the `model_max_prompt_tokens_exceeded` error code.

**What Claude Code receives today:**
```
HTTP/1.1 502 Bad Gateway
{"error":{"message":"Copilot API returned HTTP 400 Bad Request: {\"error\":{\"message\":\"prompt token count of 168929 exceeds the limit of 168000\",\"code\":\"model_max_prompt_tokens_exceeded\"}}","type":"upstream_error","code":"copilot_error"}}
```

**What Claude Code needs to see:**
```
HTTP/1.1 400 Bad Request
{"error":{"type":"invalid_request_error","message":"prompt is too long: 168929 tokens > 168000 maximum"}}
```

#### Streaming truncation handling

**`src/streaming/state.rs:handle_finish()` (lines 347-404):**

When `finish_reason == "length"` and the current block is `ToolUse`:
1. `self.tool_use_buffer.clear()` — all buffered tool_use events discarded
2. `self.truncated_openai_tool_indices.insert(idx)` — recorded internally
3. `self.block_open = false` — block never reaches the consumer
4. Emits only `MessageDelta { stop_reason: "max_tokens" }` with no content blocks

The tool_use block was **never emitted** to Claude Code — it was only buffered. So Claude Code receives `stop_reason: max_tokens` with zero tool_use blocks.

#### How Claude Code handles `stop_reason: max_tokens`

Claude Code's recovery logic in `src/services/api/claude.ts`:

```typescript
if (!needsFollowUp) {                    // line ~1062
    if (stopReason === 'max_tokens') {    // line ~1188
        // Escalate: retry with higher max_tokens (8K → 64K)
        maxOutputTokensRecoveryCount++
        maxOutputTokensOverride = upperMaxOutputTokens
    }
} else {
    // Tool result continuation path
    maxOutputTokensRecoveryCount = 0      // RESET
    maxOutputTokensOverride = undefined   // RESET
}
```

**The bug interaction:** Even though the adapter drops the tool_use block, Claude Code's Anthropic SDK streaming handler has already processed `content_block_start` events for the tool_use block (these were emitted by the model before the truncation). Wait — actually, the adapter's tool_use *buffering* means these events are never sent. Let me be precise:

With the adapter's buffering, tool_use events are held in `tool_use_buffer` and never emitted to the client. So Claude Code never sees `content_block_start` with `type: "tool_use"`. However, if there was text content before the tool call (common pattern: "Let me write that file" + Write tool call), Claude Code does see the text block and `stop_reason: max_tokens`.

**The real issue is the edge case where a prior complete tool_use block was already flushed.** When tool A completes and tool B starts, tool A's buffered events are flushed (emitted to client). If tool B is then truncated, Claude Code has already seen tool_use blocks from tool A, so `needsFollowUp = true`, and max_tokens escalation is bypassed.

For the single-tool case (the most common truncation scenario), the adapter currently works correctly — Claude Code sees only text + `max_tokens`, and escalation should fire. But the conversation logs show it doesn't always fire, suggesting there may be more nuance in Claude Code's `needsFollowUp` detection. **Option E improves this by adding explicit truncation context regardless.**

### Target State / Desired Behavior

1. When the Copilot API rejects a request for exceeding its prompt token limit, Claude Code receives a "prompt is too long" error and triggers automatic context compaction
2. When a tool call is truncated by the output token limit, Claude Code receives an informative text block and `stop_reason: max_tokens`, enabling max_tokens escalation
3. No changes required to Claude Code

---

## Problem Statement

**Observed behavior (Issue 1 — prompt too long):**
```
06:41:08.903 Sending streaming request to Copilot API model=claude-opus-4.6
06:41:09.415 ERROR Copilot API error response status=400 Bad Request
             body={"error":{"message":"prompt token count of 168929 exceeds the
             limit of 168000","code":"model_max_prompt_tokens_exceeded"}}
06:41:09.416 WARN  Copilot API error: ... error_type=upstream_error status=502
```
Claude Code receives HTTP 502, treats it as a generic upstream failure. No compaction. Next request sends equal or more tokens. Fails again.

**Observed behavior (Issue 2 — truncated tool):**
```
06:28:37.510 SSE chunk: tool_calls[1].function.arguments=".plan.md\""
             (only file_path streamed — content argument never started)
[~2 min gap while model generates]
06:30:38.622 SSE chunk: {"finish_reason":"length"}
06:30:38.622 WARN  Dropping truncated tool_use block (finish_reason="length")
```
Claude Code receives `stop_reason: max_tokens` with text content only. If escalation fires, the retry with 64K budget succeeds. If it doesn't fire (edge cases), Claude Code enters a confused loop.

**Impact:**
- Long Claude Code sessions become unusable when the conversation approaches 168K tokens
- Large file writes fail when the tool call arguments exceed the output token budget

---

## Goals and Non-Goals

### Goals

| ID | Goal | Success Criteria |
|----|------|------------------|
| G1 | Claude Code recognizes prompt-too-long errors from the adapter | `isPromptTooLongMessage()` returns `true`; `parsePromptTooLongTokenCounts()` extracts token counts |
| G2 | Truncated tool calls provide informative context | Text block emitted with truncation info; `stop_reason: max_tokens` preserved |
| G3 | No regressions in normal streaming or tool call flows | All existing tests pass; non-error paths unchanged |
| G4 | Works without Claude Code modifications | Adapter-only changes |

### Non-Goals

| ID | Non-Goal | Rationale |
|----|----------|-----------|
| NG1 | Preventing prompt-too-long at the adapter level | Future work (Option B); this fix handles recovery |
| NG2 | Modifying Claude Code source | We control only the adapter |
| NG3 | Handling all Copilot API error codes | Only `model_max_prompt_tokens_exceeded` is addressed |
| NG4 | Changing the error format for all error types | Only prompt-too-long gets special treatment |

---

## Research / Analysis

### Key Finding 1: Anthropic SDK error message construction

The Anthropic TypeScript SDK (`@anthropic-ai/sdk`) constructs `APIError.message` via a `makeMessage` static method:

```typescript
private static makeMessage(status, error, message) {
    const msg =
        error?.message ? (typeof error.message === 'string' ? error.message : JSON.stringify(error.message))
        : error ? JSON.stringify(error)
        : message;
    if (status && msg) return `${status} ${msg}`;
    // ...
}
```

The `error` parameter is the **full parsed HTTP response body**. Given the adapter's current format:

```json
{"error": {"type": "invalid_request_error", "message": "prompt is too long: 168929 tokens > 168000 maximum"}}
```

The SDK checks `body.message` → `undefined` (message is nested under `body.error.message`). Falls through to `JSON.stringify(body)`, producing:

```
400 {"error":{"type":"invalid_request_error","message":"prompt is too long: 168929 tokens > 168000 maximum"}}
```

Claude Code then checks `error.message.toLowerCase().includes('prompt is too long')` — this **matches** because the JSON-stringified body contains the substring.

**Conclusion:** The adapter's existing `{"error": {...}}` response format works. The SDK will produce an `error.message` string containing `"prompt is too long"` as long as the inner message field contains that string. No need to add a top-level `"type": "error"` wrapper.

### Key Finding 2: Claude Code's prompt-too-long regex

```typescript
// Claude Code: src/services/api/errors.ts:89-90
const match = rawMessage.match(
    /prompt is too long[^0-9]*(\d+)\s*tokens?\s*>\s*(\d+)/i,
)
```

The regex matches strings like:
- `prompt is too long: 168929 tokens > 168000 maximum` ✓
- `prompt is too long: 168929 tokens > 168000` ✓
- `Prompt is too long 100000 token > 50000` ✓

The adapter's error message must contain this pattern. The message will be embedded in the JSON-stringified body, so the regex still matches within the larger string.

### Key Finding 3: Claude Code's error.message path

Claude Code checks `error.message.toLowerCase().includes('prompt is too long')` where `error` is an `Error` (or `APIError`) instance. The SDK sets `APIError.message` from the parsed body.

Additionally, `extractNestedErrorMessage()` in `src/services/api/errorUtils.ts` can extract from two nesting levels:
1. `error.error.error.message` — Anthropic API format: body has `{"type":"error","error":{"type":"...","message":"..."}}`
2. `error.error.message` — Bedrock/proxy format: body has `{"error":{"type":"...","message":"..."}}`

The adapter's format matches pattern #2 (Bedrock/proxy), which the SDK supports.

### Key Finding 4: Token count parsing from Copilot error

The Copilot API returns:
```json
{"error":{"message":"prompt token count of 168929 exceeds the limit of 168000","code":"model_max_prompt_tokens_exceeded"}}
```

We need to parse `168929` (actual) and `168000` (limit) from this message and reformat as:
```
prompt is too long: 168929 tokens > 168000 maximum
```

Parsing regex for the Copilot format:
```
/prompt token count of (\d+) exceeds the limit of (\d+)/
```

### Key Finding 5: Streaming state machine tool_use buffering

The `StreamingState` in `src/streaming/state.rs` buffers all tool_use events. They are only flushed (sent to client) when:
- A new tool call starts (the previous one is implicitly complete)
- `finish_reason` is `"tool_calls"` or `"stop"` (normal completion)
- `finalize()` is called (stream ends without finish_reason)

When `finish_reason: "length"` arrives with an open tool_use:
- Buffer is cleared (events never sent to client)
- The tool_use block index is recorded in `truncated_openai_tool_indices`
- Only `MessageDelta { stop_reason: "max_tokens" }` is emitted

**Key implication for Option E:** Since the tool_use block was never emitted to the client, we can safely emit a text block instead. The client never knew a tool_use was attempted. From Claude Code's perspective, the response will contain text content blocks (including the truncation notice) and `stop_reason: max_tokens`.

### Key Finding 6: State tracking for "any complete tool_use emitted"

The `StreamingState` struct tracks:
- `current_block_index: u32` — increments for each emitted block
- `tool_use_buffer: Vec<StreamEvent>` — current tool's buffered events
- `truncated_openai_tool_indices: HashSet<u32>` — which tools were truncated

Currently there's no field that tracks "at least one tool_use block was fully emitted to the client." We need to add one for Option E to decide whether the truncation notice text block is the only content or accompanies earlier tool_use blocks.

Actually, looking more carefully: when a tool_use block is flushed, the events include `ContentBlockStart` with a `ResponseContentBlock::ToolUse`. We can add a `bool` flag `has_emitted_tool_use` that is set to `true` whenever `flush_tool_use_buffer()` returns non-empty events.

---

## Proposed Design

### Option A: Translate `model_max_prompt_tokens_exceeded` to Anthropic error format

#### 1. Parse Copilot API 400 error body

**File: `src/copilot/client.rs`**

Add a helper function to detect and parse the prompt-too-long error from the Copilot API response body:

```rust
/// Parse a Copilot API error body for `model_max_prompt_tokens_exceeded`.
///
/// Returns `(actual_tokens, limit_tokens)` if the error matches.
///
/// Expected format:
/// ```json
/// {"error":{"message":"prompt token count of 168929 exceeds the limit of 168000",
///           "code":"model_max_prompt_tokens_exceeded"}}
/// ```
fn parse_prompt_too_long(body: &str) -> Option<(u32, u32)> {
    let parsed: serde_json::Value = serde_json::from_str(body).ok()?;
    let code = parsed
        .get("error")?
        .get("code")?
        .as_str()?;

    if code != "model_max_prompt_tokens_exceeded" {
        return None;
    }

    let message = parsed
        .get("error")?
        .get("message")?
        .as_str()?;

    // Parse "prompt token count of 168929 exceeds the limit of 168000"
    let re = regex::Regex::new(
        r"prompt token count of (\d+) exceeds the limit of (\d+)"
    ).ok()?;
    let caps = re.captures(message)?;
    let actual: u32 = caps.get(1)?.as_str().parse().ok()?;
    let limit: u32 = caps.get(2)?.as_str().parse().ok()?;
    Some((actual, limit))
}
```

**Note on `regex` dependency:** The `regex` crate is already a transitive dependency via `tracing-subscriber` and other crates. We can add it as a direct dependency, or use a simpler string-parsing approach to avoid the new dependency. The string approach would use `str::find` and `str::parse`:

```rust
fn parse_prompt_too_long(body: &str) -> Option<(u32, u32)> {
    let parsed: serde_json::Value = serde_json::from_str(body).ok()?;
    let error_obj = parsed.get("error")?;

    let code = error_obj.get("code")?.as_str()?;
    if code != "model_max_prompt_tokens_exceeded" {
        return None;
    }

    let message = error_obj.get("message")?.as_str()?;

    // Parse "prompt token count of N exceeds the limit of M"
    let actual_start = message.find("prompt token count of ")? + "prompt token count of ".len();
    let actual_end = message[actual_start..].find(' ')? + actual_start;
    let actual: u32 = message[actual_start..actual_end].parse().ok()?;

    let limit_start = message.find("exceeds the limit of ")? + "exceeds the limit of ".len();
    let limit: u32 = message[limit_start..].trim().parse().ok()?;

    Some((actual, limit))
}
```

Either approach works. The string-parsing version avoids adding `regex` as a direct dependency.

#### 2. Update `handle_error_response` to detect prompt-too-long

**File: `src/copilot/client.rs` — `handle_error_response()` (lines 93-112)**

Current:
```rust
async fn handle_error_response(response: reqwest::Response) -> AppError {
    let status = response.status();
    if status.as_u16() == 429 { /* ... */ }
    let body = response.text().await.unwrap_or_default();
    tracing::error!(status = %status, body = %body, "Copilot API error response");
    AppError::CopilotError(format!("Copilot API returned HTTP {status}: {body}"))
}
```

Updated:
```rust
async fn handle_error_response(response: reqwest::Response) -> AppError {
    let status = response.status();

    if status.as_u16() == 429 {
        let retry_after = Self::parse_retry_after(&response);
        tracing::warn!(retry_after_secs = retry_after, "Rate limited by Copilot API");
        return AppError::RateLimited(retry_after);
    }

    let body = response.text().await.unwrap_or_default();
    tracing::error!(status = %status, body = %body, "Copilot API error response");

    // Detect prompt-too-long errors and translate to Anthropic format.
    if status.as_u16() == 400 {
        if let Some((actual, limit)) = parse_prompt_too_long(&body) {
            tracing::info!(
                actual_tokens = actual,
                limit_tokens = limit,
                "Translating prompt-too-long error to Anthropic format"
            );
            return AppError::PromptTooLong {
                actual_tokens: actual,
                limit_tokens: limit,
            };
        }
    }

    AppError::CopilotError(format!("Copilot API returned HTTP {status}: {body}"))
}
```

#### 3. Add `PromptTooLong` error variant

**File: `src/error.rs`**

Add new variant to `AppError`:

```rust
#[derive(thiserror::Error, Debug)]
pub enum AppError {
    // ... existing variants ...

    #[error("prompt is too long: {actual_tokens} tokens > {limit_tokens} maximum")]
    PromptTooLong {
        actual_tokens: u32,
        limit_tokens: u32,
    },
}
```

**The `#[error(...)]` format string is critical.** It produces:
```
prompt is too long: 168929 tokens > 168000 maximum
```

This matches Claude Code's regex `/prompt is too long[^0-9]*(\d+)\s*tokens?\s*>\s*(\d+)/i` exactly:
- `prompt is too long` ✓ (case-insensitive match)
- `: ` matches `[^0-9]*` ✓
- `168929` matches `(\d+)` → captured as group 1 (actualTokens) ✓
- ` tokens > ` matches `\s*tokens?\s*>\s*` ✓
- `168000` matches `(\d+)` → captured as group 2 (limitTokens) ✓

#### 4. Map `PromptTooLong` to HTTP response

**File: `src/error.rs` — `IntoResponse` impl**

Add to the match arm:

```rust
AppError::PromptTooLong { actual_tokens, limit_tokens } => (
    StatusCode::BAD_REQUEST,
    json!({
        "error": {
            "message": self.to_string(),
            "type": "invalid_request_error",
            "code": "prompt_too_long"
        }
    }),
),
```

This produces:
```
HTTP/1.1 400 Bad Request
{
    "error": {
        "message": "prompt is too long: 168929 tokens > 168000 maximum",
        "type": "invalid_request_error",
        "code": "prompt_too_long"
    }
}
```

The Anthropic SDK will:
1. Receive HTTP 400 → create `BadRequestError` (subclass of `APIError`)
2. Parse the body and store it as `error.error`
3. Set `error.message` = `makeMessage(400, body, undefined)`
4. Since `body.message` is `undefined` (message is at `body.error.message`), `makeMessage` falls through to `JSON.stringify(body)`:
   ```
   400 {"error":{"message":"prompt is too long: 168929 tokens > 168000 maximum","type":"invalid_request_error","code":"prompt_too_long"}}
   ```
5. Claude Code checks `error.message.toLowerCase().includes('prompt is too long')` → **true** ✓
6. Claude Code parses token counts via regex → `actualTokens: 168929`, `limitTokens: 168000` ✓

#### 5. Update `error_type()` method

**File: `src/error.rs` — `error_type()` method**

```rust
pub fn error_type(&self) -> &'static str {
    match self {
        // ... existing arms ...
        AppError::PromptTooLong { .. } => "invalid_request_error",
    }
}
```

---

### Option E: Emit text block for truncated tool calls

#### 1. Add state tracking for emitted tool_use blocks

**File: `src/streaming/state.rs` — `StreamingState` struct**

Add a new field:

```rust
pub struct StreamingState {
    // ... existing fields ...

    /// Whether at least one complete tool_use block has been flushed
    /// (emitted to the consumer). Used to decide the content of the
    /// truncation notice: when `false`, the truncation text is the only
    /// content the consumer has seen, maximizing the chance that Claude
    /// Code's max_tokens escalation fires.
    has_emitted_tool_use: bool,
}
```

Initialize in `new()`:
```rust
has_emitted_tool_use: false,
```

Set in `flush_tool_use_buffer()`:
```rust
fn flush_tool_use_buffer(&mut self) -> Vec<StreamEvent> {
    if self.tool_use_buffer.is_empty() {
        return Vec::new();
    }
    self.has_emitted_tool_use = true;   // NEW
    std::mem::take(&mut self.tool_use_buffer)
}
```

#### 2. Modify `handle_finish()` to emit truncation text block

**File: `src/streaming/state.rs` — `handle_finish()` (lines 347-404)**

Current truncation path (lines 350-364):
```rust
if reason == "length" && self.current_block_type == Some(ContentBlockType::ToolUse) {
    tracing::warn!(/* ... */ "Dropping truncated tool_use block (finish_reason=\"length\")");
    self.tool_use_buffer.clear();
    if let Some(oi_idx) = self.current_openai_tool_index {
        self.truncated_openai_tool_indices.insert(oi_idx);
    }
    self.block_open = false;
    self.current_block_type = None;
}
```

Updated truncation path:
```rust
if reason == "length" && self.current_block_type == Some(ContentBlockType::ToolUse) {
    // Retrieve the tool name before clearing the buffer (for the notice).
    let tool_name = self
        .current_openai_tool_index
        .and_then(|idx| self.tool_call_names.get(&idx))
        .cloned()
        .unwrap_or_else(|| "unknown".to_string());

    tracing::warn!(
        block_index = self.current_block_index,
        openai_tool_index = ?self.current_openai_tool_index,
        tool_name = %tool_name,
        "Dropping truncated tool_use block (finish_reason=\"length\")"
    );

    // Discard the incomplete tool_use events.
    self.tool_use_buffer.clear();
    if let Some(oi_idx) = self.current_openai_tool_index {
        self.truncated_openai_tool_indices.insert(oi_idx);
    }

    // The tool_use block was never emitted. Reset block state.
    self.block_open = false;
    self.current_block_type = None;

    // Emit a text block explaining the truncation. This gives Claude
    // Code context about what happened and, when no other tool_use
    // blocks were emitted, ensures `needsFollowUp` is false so the
    // max_tokens escalation path fires.
    let notice = format!(
        "[Tool call to \"{}\" was truncated due to output token limit]",
        tool_name
    );

    events.push(StreamEvent::ContentBlockStart {
        index: self.current_block_index,
        content_block: ResponseContentBlock::text(String::new()),
    });
    events.push(StreamEvent::ContentBlockDelta {
        index: self.current_block_index,
        delta: ContentDelta::Text(TextDelta {
            delta_type: "text_delta".to_string(),
            text: notice,
        }),
    });
    events.push(StreamEvent::ContentBlockStop {
        index: self.current_block_index,
    });
    self.current_block_index += 1;
}
```

**What this achieves:**
- Claude Code sees a text block `[Tool call to "Write" was truncated due to output token limit]` followed by `stop_reason: max_tokens`
- If no prior tool_use blocks were emitted (`!has_emitted_tool_use`), the response contains only text → `needsFollowUp = false` → max_tokens escalation fires
- If prior tool_use blocks were emitted, Claude Code will process those tool calls first. The truncation notice is informational — the model will see it on the next turn and can decide whether to retry
- The notice text is clearly a system annotation (square brackets), not model-generated content

#### 3. Handle edge case: text block already open

If the model was generating text *and then* started a tool call that got truncated, there's already an open text block (closed when the tool call started). The new text block is separate.

If the model was generating *only* tool calls (no text), there's no prior text block. The truncation notice is the first and only content block. `message_start` was already emitted (by the tool call's first chunk), so the response is:

```
message_start
content_block_start (index=0, text)
content_block_delta (index=0, "[Tool call to "Write" was truncated...]")
content_block_stop (index=0)
message_delta (stop_reason: "max_tokens")
message_stop
```

This is a valid Anthropic streaming response.

---

## File Changes Summary

| File | Change | Description |
|------|--------|-------------|
| `src/error.rs` | Modified | Add `PromptTooLong` variant, HTTP 400 mapping, `error_type()` arm |
| `src/copilot/client.rs` | Modified | Add `parse_prompt_too_long()`, update `handle_error_response()` |
| `src/streaming/state.rs` | Modified | Add `has_emitted_tool_use` field, emit text block on truncation |
| `tests/unit/error_tests.rs` | Modified | Add test for `PromptTooLong` error format |
| `tests/unit/streaming_tests.rs` | Modified | Update truncation tests to expect text block |
| `tests/integration/error_tests.rs` | Modified | Add integration test for prompt-too-long translation |

---

## Testing Strategy

### Unit Tests

#### Error translation tests (`tests/unit/error_tests.rs`)

```rust
#[tokio::test]
async fn prompt_too_long_returns_400_with_anthropic_format() {
    let (status, json) = error_to_parts(AppError::PromptTooLong {
        actual_tokens: 168929,
        limit_tokens: 168000,
    }).await;
    assert_eq!(status, 400);
    assert_eq!(json["error"]["type"], "invalid_request_error");
    assert_eq!(json["error"]["code"], "prompt_too_long");
    let message = json["error"]["message"].as_str().unwrap();
    assert_eq!(message, "prompt is too long: 168929 tokens > 168000 maximum");
}

#[test]
fn prompt_too_long_message_matches_claude_code_regex() {
    let err = AppError::PromptTooLong {
        actual_tokens: 168929,
        limit_tokens: 168000,
    };
    let message = err.to_string();

    // Simulate what the Anthropic SDK does: JSON.stringify the body
    let body = serde_json::json!({
        "error": {
            "message": message,
            "type": "invalid_request_error",
            "code": "prompt_too_long"
        }
    });
    let sdk_message = format!("400 {}", serde_json::to_string(&body).unwrap());

    // Claude Code's regex
    let re = regex::Regex::new(
        r"(?i)prompt is too long[^0-9]*(\d+)\s*tokens?\s*>\s*(\d+)"
    ).unwrap();
    let caps = re.captures(&sdk_message).expect("regex must match");
    assert_eq!(caps.get(1).unwrap().as_str(), "168929");
    assert_eq!(caps.get(2).unwrap().as_str(), "168000");
}
```

#### Copilot error parsing tests (`tests/unit/copilot_client_tests.rs`)

```rust
#[test]
fn parse_prompt_too_long_valid_body() {
    let body = r#"{"error":{"message":"prompt token count of 168929 exceeds the limit of 168000","code":"model_max_prompt_tokens_exceeded"}}"#;
    let result = parse_prompt_too_long(body);
    assert_eq!(result, Some((168929, 168000)));
}

#[test]
fn parse_prompt_too_long_different_numbers() {
    let body = r#"{"error":{"message":"prompt token count of 50000 exceeds the limit of 32000","code":"model_max_prompt_tokens_exceeded"}}"#;
    let result = parse_prompt_too_long(body);
    assert_eq!(result, Some((50000, 32000)));
}

#[test]
fn parse_prompt_too_long_wrong_code() {
    let body = r#"{"error":{"message":"something else","code":"other_error"}}"#;
    let result = parse_prompt_too_long(body);
    assert_eq!(result, None);
}

#[test]
fn parse_prompt_too_long_invalid_json() {
    let result = parse_prompt_too_long("not json");
    assert_eq!(result, None);
}

#[test]
fn parse_prompt_too_long_missing_fields() {
    let body = r#"{"error":{"code":"model_max_prompt_tokens_exceeded"}}"#;
    let result = parse_prompt_too_long(body);
    assert_eq!(result, None);
}
```

#### Streaming truncation tests (`tests/unit/streaming_tests.rs`)

Update existing tests and add new ones:

```rust
/// When a single tool call is truncated by finish_reason="length",
/// a text block with a truncation notice should be emitted, followed
/// by message_delta("max_tokens").
#[test]
fn tool_call_truncated_by_length_emits_text_notice() {
    let mut state = StreamingState::new(HashMap::new());

    // Tool call starts — events are buffered, only message_start returned
    let events = state.process_chunk(&tool_call_start_chunk(
        "c1", "m1", 0, "call_abc", "Write", "{\"file_path",
    ));
    assert_eq!(events.len(), 1); // only message_start
    assert_message_start(&events[0]);

    // More arguments — still buffered
    let events = state.process_chunk(&tool_call_args_chunk(
        "c1", "m1", 0, "\": \"test.md\"",
    ));
    assert_eq!(events.len(), 0);

    // Truncated by length — text notice emitted instead of tool_use
    let events = state.process_chunk(&finish_chunk("c1", "m1", "length"));
    // Expected: text block_start + text delta + block_stop + message_delta
    assert_eq!(events.len(), 4);
    assert_text_block_start(&events[0], 0);
    assert_text_delta(
        &events[1], 0,
        "[Tool call to \"Write\" was truncated due to output token limit]"
    );
    assert_block_stop(&events[2], 0);
    assert_message_delta(&events[3], "max_tokens");

    // Verify truncation was tracked
    assert!(state.truncated_openai_tool_indices().contains(&0));

    let events = state.finalize();
    assert_eq!(events.len(), 1);
    assert_message_stop(&events[0]);
}

/// Text block followed by a tool call that gets truncated.
/// Text emitted normally, truncation notice as separate block.
#[test]
fn text_then_tool_truncated_emits_text_notice() {
    let mut state = StreamingState::new(HashMap::new());

    // Text streams normally
    let events = state.process_chunk(&text_chunk("c1", "m1", "Let me write that.", None));
    assert_eq!(events.len(), 3);

    // Tool call starts — text block closed, tool buffered
    let events = state.process_chunk(&tool_call_start_chunk(
        "c1", "m1", 0, "call_1", "Write", "{\"file",
    ));
    assert_eq!(events.len(), 1);
    assert_block_stop(&events[0], 0);

    // Truncated — text notice emitted
    let events = state.process_chunk(&finish_chunk("c1", "m1", "length"));
    assert_eq!(events.len(), 4);
    assert_text_block_start(&events[0], 1); // index 1 (after text block 0)
    assert_text_delta(
        &events[1], 1,
        "[Tool call to \"Write\" was truncated due to output token limit]"
    );
    assert_block_stop(&events[2], 1);
    assert_message_delta(&events[3], "max_tokens");
}

/// Two parallel tools: first complete, second truncated.
/// First tool emitted normally, truncation notice for second.
#[test]
fn first_tool_complete_second_truncated_emits_notice() {
    let mut state = StreamingState::new(HashMap::new());

    // First tool — buffered
    let events = state.process_chunk(&tool_call_start_chunk(
        "c1", "m1", 0, "call_a", "Read", "{\"path\":\"a.rs\"}",
    ));
    assert_eq!(events.len(), 1);
    assert_message_start(&events[0]);

    // Second tool starts — first tool flushed (complete)
    let events = state.process_chunk(&tool_call_start_chunk(
        "c1", "m1", 1, "call_b", "Write", "{\"file",
    ));
    assert_eq!(events.len(), 3);
    assert_tool_use_block_start(&events[0], 0, "call_a", "Read");
    assert_input_json_delta(&events[1], 0, "{\"path\":\"a.rs\"}");
    assert_block_stop(&events[2], 0);

    // Second tool truncated — notice emitted
    let events = state.process_chunk(&finish_chunk("c1", "m1", "length"));
    assert_eq!(events.len(), 4);
    assert_text_block_start(&events[0], 1);
    assert_text_delta(
        &events[1], 1,
        "[Tool call to \"Write\" was truncated due to output token limit]"
    );
    assert_block_stop(&events[2], 1);
    assert_message_delta(&events[3], "max_tokens");

    // Only second tool was truncated
    assert!(!state.truncated_openai_tool_indices().contains(&0));
    assert!(state.truncated_openai_tool_indices().contains(&1));
}
```

### Integration Tests

1. **Mock Copilot API returning 400 `model_max_prompt_tokens_exceeded`:**
   - Send a request through the adapter
   - Verify HTTP 400 response with correct error format
   - Verify the message matches Claude Code's regex

2. **Mock Copilot API streaming response ending with `finish_reason: "length"` mid-tool-call:**
   - Start streaming with a tool call that gets truncated
   - Verify the SSE stream contains the text notice block
   - Verify `stop_reason: max_tokens`
   - Verify no tool_use blocks in the stream

### Manual E2E Tests

1. Start a long Claude Code conversation until context exceeds 168K tokens
2. Verify Claude Code receives "prompt too long" error and triggers compaction
3. Request writing a large file that exceeds the output token budget
4. Verify Claude Code escalates max_tokens and retries successfully

---

## Risk Assessment

| Risk | Impact | Probability | Mitigation |
|------|--------|-------------|------------|
| Anthropic SDK doesn't match `prompt is too long` in JSON-stringified body | High | Low | Unit test verifies exact SDK behavior; the regex is case-insensitive and matches substrings |
| Copilot API changes error message format | Medium | Low | `parse_prompt_too_long` returns `None` for unrecognized formats → falls through to generic `CopilotError` (existing behavior) |
| Truncation text block confuses the model on retry | Low | Medium | Square-bracket format is clearly system-level; model has seen similar patterns |
| `has_emitted_tool_use` tracking adds complexity | Low | Low | Simple boolean, tested in unit tests |
| Other Copilot API 400 errors wrongly matched | Low | Very Low | We check the `code` field specifically, not just the message |
| Edge case: tool_use block with no recorded name | Low | Low | Defaults to `"unknown"` in the notice text |

---

## Success Criteria

1. **Prompt too long** — Claude Code's `isPromptTooLongMessage()` returns `true` for the translated error
2. **Token parsing** — `parsePromptTooLongTokenCounts()` extracts correct `actualTokens` and `limitTokens`
3. **Tool truncation** — Text notice block is emitted; `stop_reason: max_tokens` preserved
4. **No regressions** — All existing streaming and error tests pass
5. **Conversation logging** — Truncated tool calls are still excluded from conversation logs (existing behavior)

---

## Design Decisions

| Decision | Rationale |
|----------|-----------|
| Use `{"error": {...}}` format (not `{"type": "error", "error": {...}}`) | The adapter's existing format matches the Bedrock/proxy pattern, which the Anthropic SDK already handles. Changing ALL error responses to add `"type": "error"` would be a larger change for no benefit. |
| Parse Copilot error by `code` field, not message text | The `code` field is a stable identifier; the message text could change between API versions. |
| Use string parsing instead of `regex` crate | Avoids adding a direct dependency. The Copilot error message has a fixed format. If it changes, the parser gracefully returns `None`. |
| Emit text block for truncation (not partial tool_use) | Safety: partial tool_use blocks could cause Claude Code to execute incomplete tool calls. Text blocks are inert. |
| Use `[square brackets]` for truncation notice | Matches common system-message formatting conventions. Clearly distinguishes from model-generated text. |
| Don't change behavior when prior tool_use blocks exist | When tool A completed and tool B was truncated, Claude Code will process tool A's result normally. The truncation notice serves as context for the model's next turn, not as a recovery trigger. |

---

## Open Questions

| # | Question | Status |
|---|----------|--------|
| 1 | What is the per-model prompt token limit on the Copilot API? Is 168K the same for all Claude models? | Open — observed 168K for claude-opus-4.6 |
| 2 | Should we also translate other Copilot 400 errors (e.g., content policy violations)? | Deferred — only prompt-too-long is addressed in this design |
| 3 | Should Option B (pre-flight token validation) be implemented as a follow-up? | Deferred — would prevent the wasted round-trip |
| 4 | Should the truncation notice include partial argument data (e.g., file_path)? | No — keeping it minimal reduces confusion |

---

## References

### copilot-adapter
- `src/error.rs` — AppError enum and HTTP status mapping (lines 1-166)
- `src/copilot/client.rs` — `handle_error_response()` (lines 93-112)
- `src/streaming/state.rs` — `handle_finish()` (lines 347-404), `StreamingState` struct (lines 31-67)
- `src/handlers/messages.rs` — `handle_native_tools_streaming()` (lines 1113-1332)
- `tests/unit/error_tests.rs` — Existing error format tests
- `tests/unit/streaming_tests.rs` — Existing streaming/truncation tests

### Claude Code
- `src/services/api/errors.ts` — `isPromptTooLongMessage()` (lines 64-77), regex (lines 89-90), prompt-too-long handler (lines 560-574)
- `src/services/api/errorUtils.ts` — `extractNestedErrorMessage()` (lines 169-198), SDK error shapes comment (lines 132-142)
- `src/services/api/claude.ts` — max_tokens escalation (line ~1062), `needsFollowUp` guard, `model_context_window_exceeded` handling (lines 2279-2292)

### Anthropic SDK
- `@anthropic-ai/sdk` — `APIError.makeMessage()` constructs `error.message` from the HTTP response body; checks `body.message` first, then `JSON.stringify(body)`

---

## Appendix

### A: Full error flow with fix applied

```
Claude Code sends: POST /v1/messages (168929 prompt tokens)
  ↓
copilot-adapter forwards to Copilot API
  ↓
Copilot API returns: HTTP 400
  {"error":{"message":"prompt token count of 168929 exceeds the limit of 168000",
            "code":"model_max_prompt_tokens_exceeded"}}
  ↓
copilot-adapter: parse_prompt_too_long() → Some((168929, 168000))
  ↓
copilot-adapter returns: HTTP 400
  {"error":{"message":"prompt is too long: 168929 tokens > 168000 maximum",
            "type":"invalid_request_error","code":"prompt_too_long"}}
  ↓
Anthropic SDK: creates BadRequestError (APIError subclass)
  error.status = 400
  error.error = {"error":{"message":"prompt is too long: 168929 tokens > 168000 maximum",...}}
  error.message = '400 {"error":{"message":"prompt is too long: 168929 tokens > 168000 maximum",...}}'
  ↓
Claude Code: error.message.toLowerCase().includes('prompt is too long') → TRUE
  ↓
Claude Code: parsePromptTooLongTokenCounts(error.message) → {actualTokens: 168929, limitTokens: 168000}
  ↓
Claude Code: triggers reactive compaction with gap = 168929 - 168000 = 929 tokens
```

### B: Full truncation flow with fix applied

```
Claude Code sends: POST /v1/messages (with Write tool, max_tokens=8000)
  ↓
copilot-adapter: StreamingState processes chunks:
  chunk 1: content="Let me write that file" → text block emitted
  chunk 2: tool_call[0] name="Write" → buffered
  chunk 3: tool_call[0] args='{"file_path":"...' → buffered
  ...
  chunk N: finish_reason="length" → TRUNCATION DETECTED
  ↓
StreamingState.handle_finish("length"):
  1. Clear tool_use_buffer
  2. Record truncated index
  3. Emit text block: "[Tool call to "Write" was truncated due to output token limit]"
  4. Emit MessageDelta { stop_reason: "max_tokens" }
  ↓
Claude Code receives SSE stream:
  message_start
  content_block_start (index=0, text)     → "Let me write that file"
  content_block_delta (index=0)
  content_block_stop (index=0)
  content_block_start (index=1, text)     → truncation notice
  content_block_delta (index=1)
  content_block_stop (index=1)
  message_delta (stop_reason: max_tokens)
  message_stop
  ↓
Claude Code: No tool_use blocks → needsFollowUp = false
  stopReason = "max_tokens" → escalation fires
  maxOutputTokensOverride = 64000
  ↓
Claude Code retries with max_tokens=64000 → Write tool call completes successfully
```
