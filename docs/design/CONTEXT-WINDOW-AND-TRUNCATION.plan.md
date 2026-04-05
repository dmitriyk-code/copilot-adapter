# Context Window Enforcement & Truncated Tool Recovery — Implementation Plan

**Status:** Not Started
**Date:** 2026-04-05
**Based on:** [CONTEXT-WINDOW-AND-TRUNCATION.design.md](./CONTEXT-WINDOW-AND-TRUNCATION.design.md)
**Prerequisite:** None
**Estimated Time:** 1-2 days

---

## Executive Summary

The copilot-adapter has three related issues that cause Claude Code sessions to fail or underperform during long conversations, large file writes, or 1M context usage. This plan implements three targeted fixes designed in the companion design document:

1. **Option A — Prompt-too-long error translation:** Translate GitHub Copilot's `model_max_prompt_tokens_exceeded` HTTP 400 error into an Anthropic-format `invalid_request_error` with a message matching Claude Code's prompt-too-long regex, so Claude Code triggers automatic context compaction.
2. **Option C — 1M context model activation:** Detect Claude Code's `anthropic-beta: context-1m-*` HTTP header and append `-1m` to the normalized Copilot model name (e.g., `claude-opus-4.6` → `claude-opus-4.6-1m`), enabling true 1M context windows via the Copilot API's distinct model ID.
3. **Option E — Truncated tool call recovery:** When a tool call is truncated by `finish_reason: "length"`, emit a descriptive text content block instead of silently dropping the incomplete tool_use block, so Claude Code sees a text-only response and can fire its max_tokens escalation logic (8K → 64K retry).

This plan implements:
- New `PromptTooLong` error variant with Anthropic-compatible HTTP 400 response
- Copilot API error body parser for `model_max_prompt_tokens_exceeded`
- `anthropic-beta` header extraction and `context-1m-*` detection in the messages handler
- Model name `-1m` suffix appending for 1M context activation
- Streaming state machine changes to emit truncation notice text blocks
- New `has_emitted_tool_use` tracking field in `StreamingState`
- Comprehensive unit, integration, and manual E2E tests

**Total estimated time:** 1.5-2.5 days

---

## Background

### Current State

- **Error handling (`src/copilot/client.rs`, lines 91-112):** All non-429 Copilot API errors become `AppError::CopilotError` → HTTP 502 `upstream_error`. No special handling for HTTP 400 or `model_max_prompt_tokens_exceeded`.
- **Streaming truncation (`src/streaming/state.rs`, lines 336-404):** When `finish_reason == "length"` mid-tool-call, the adapter clears the `tool_use_buffer`, records the truncated index, and emits only `MessageDelta { stop_reason: "max_tokens" }` with no content blocks.
- **Error types (`src/error.rs`, lines 13-37):** 8 existing variants: `NotAuthenticated`, `TokenExpired`, `GitHubError`, `CopilotError`, `RateLimited`, `InvalidRequest`, `ModelNotFound`, `Internal`.
- **`StreamingState` struct (`src/streaming/state.rs`, lines 32-68):** 13 fields. No tracking of whether complete tool_use blocks have been emitted.
- **`regex` crate:** Already a direct dependency in `Cargo.toml` (line 27: `regex = "1"`).
- **Messages handler (`src/handlers/messages.rs`, line 38-41):** Extracts only `State` and `Json<AnthropicRequest>`. Does not extract HTTP headers. No access to `anthropic-beta` or any other header.
- **`AnthropicRequest` struct (`src/anthropic/types.rs`, lines 228-252):** No `betas` field. The Anthropic SDK sends betas as the `anthropic-beta` HTTP header, not in the JSON body.
- **`model_mapper.rs` (lines 18-85):** Has context marker preservation logic for `-1m`/`-200k` in model names, but this code is unreachable from Claude Code — Claude Code strips `[1m]` before sending and uses the beta header instead.
- **Copilot API models:** Live query to `GET https://api.githubcopilot.com/models` confirms `claude-opus-4.6-1m` exists as a distinct model ID alongside `claude-opus-4.6`.

### Target State

- Copilot API 400 `model_max_prompt_tokens_exceeded` → adapter returns HTTP 400 with `"type": "invalid_request_error"` and message `"prompt is too long: N tokens > M maximum"` → Claude Code triggers context compaction.
- `anthropic-beta: context-1m-*` header → adapter appends `-1m` to normalized model name → Copilot API receives 1M model ID (e.g., `claude-opus-4.6-1m`) → 1M context window activated.
- Tool call truncated by `finish_reason: "length"` → adapter emits a text block `[Tool call to "ToolName" was truncated due to output token limit]` + `stop_reason: max_tokens` → Claude Code fires max_tokens escalation.
- All existing tests continue to pass. Normal streaming and error paths unchanged.

---

## Goals and Non-Goals

### Goals

| ID | Goal | Success Criteria |
|----|------|------------------|
| G1 | Claude Code recognizes prompt-too-long errors from the adapter | `isPromptTooLongMessage()` returns `true`; `parsePromptTooLongTokenCounts()` extracts token counts |
| G2 | Truncated tool calls provide informative context | Text block emitted with truncation info; `stop_reason: max_tokens` preserved |
| G3 | No regressions in normal streaming or tool call flows | All existing tests pass; non-error paths unchanged |
| G4 | Works without Claude Code modifications | Adapter-only changes |
| G5 | 1M context models are activated when Claude Code requests them | `anthropic-beta: context-1m-*` → model name has `-1m` suffix → Copilot API receives 1M model ID |

### Non-Goals

| ID | Non-Goal | Rationale |
|----|----------|-----------|
| NG1 | Preventing prompt-too-long at the adapter level (pre-flight validation) | Future work (Option B in design doc); this fix handles recovery |
| NG2 | Modifying Claude Code source | We control only the adapter |
| NG3 | Handling all Copilot API error codes | Only `model_max_prompt_tokens_exceeded` is addressed |
| NG4 | Changing the error format for all error types | Only prompt-too-long gets special treatment |

---

## Implementation Plan

### Epic 1: Prompt-Too-Long Error Translation (Day 1, ~0.5 day)

**Status:** Not Started

**Objective:** Translate GitHub Copilot's `model_max_prompt_tokens_exceeded` error into an Anthropic-compatible `invalid_request_error` with a message that matches Claude Code's prompt-too-long detection regex.

#### Task 1.1: Add `PromptTooLong` error variant

**File:** `src/error.rs` (MODIFIED)

**Description:** Add a new `PromptTooLong` variant to `AppError` with `actual_tokens` and `limit_tokens` fields. The `#[error(...)]` format string must produce `"prompt is too long: N tokens > M maximum"` — this exact format matches Claude Code's regex `/prompt is too long[^0-9]*(\d+)\s*tokens?\s*>\s*(\d+)/i`.

**Implementation:**

Add variant after existing variants (~line 37):
```rust
#[error("prompt is too long: {actual_tokens} tokens > {limit_tokens} maximum")]
PromptTooLong {
    actual_tokens: u32,
    limit_tokens: u32,
},
```

Add HTTP response mapping in `IntoResponse` impl (~line 39-145):
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

Add `error_type()` arm (~line 147-159):
```rust
AppError::PromptTooLong { .. } => "invalid_request_error",
```

**Acceptance Criteria:**
- [ ] `PromptTooLong` variant compiles with correct `#[error]` format
- [ ] `IntoResponse` returns HTTP 400 with `"type": "invalid_request_error"`
- [ ] `error_type()` returns `"invalid_request_error"`
- [ ] Error message exactly matches: `"prompt is too long: {actual} tokens > {limit} maximum"`

**Notes:** The error message format is **critical** — it must match Claude Code's regex. The regex requires: the literal `prompt is too long`, followed by non-digits, followed by the actual token count, `tokens`, `>`, and the limit count.

#### Task 1.2: Add `parse_prompt_too_long()` helper function

**File:** `src/copilot/client.rs` (MODIFIED)

**Description:** Add a helper function to detect and parse the `model_max_prompt_tokens_exceeded` error from a Copilot API response body. Uses string parsing (not regex) to extract the actual and limit token counts from the Copilot error message format.

**Implementation:**

Add as a module-level function (above `impl CopilotClient`):
```rust
/// Parse a Copilot API error body for `model_max_prompt_tokens_exceeded`.
///
/// Returns `(actual_tokens, limit_tokens)` if the error matches the expected format:
/// ```json
/// {"error":{"message":"prompt token count of 168929 exceeds the limit of 168000",
///           "code":"model_max_prompt_tokens_exceeded"}}
/// ```
///
/// Returns `None` for any unrecognized format, allowing fallback to generic error handling.
pub fn parse_prompt_too_long(body: &str) -> Option<(u32, u32)> {
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

**Acceptance Criteria:**
- [ ] Parses `(168929, 168000)` from the standard Copilot error format
- [ ] Returns `None` for wrong `code` field
- [ ] Returns `None` for invalid JSON
- [ ] Returns `None` for missing fields
- [ ] Returns `None` for unparseable message text
- [ ] Function is `pub` so unit tests can access it

**Notes:** String parsing is preferred over regex to keep the implementation simple and avoid adding a new usage pattern (even though `regex` is a direct dependency). The Copilot error message has a fixed format; if it changes, the parser gracefully returns `None` and the error falls through to the existing generic `CopilotError` path.

#### Task 1.3: Update `handle_error_response()` to detect prompt-too-long

**File:** `src/copilot/client.rs` (MODIFIED)

**Description:** Modify the existing `handle_error_response()` method (lines 91-112) to check for HTTP 400 with `model_max_prompt_tokens_exceeded` before falling through to the generic `CopilotError` path.

**Implementation:**

```rust
// Before (lines 91-112):
async fn handle_error_response(response: reqwest::Response) -> AppError {
    let status = response.status();
    if status.as_u16() == 429 {
        let retry_after = Self::parse_retry_after(&response);
        tracing::warn!(retry_after_secs = retry_after, "Rate limited by Copilot API");
        return AppError::RateLimited(retry_after);
    }
    let body = response.text().await.unwrap_or_default();
    tracing::error!(status = %status, body = %body, "Copilot API error response");
    AppError::CopilotError(format!("Copilot API returned HTTP {status}: {body}"))
}

// After:
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

**Acceptance Criteria:**
- [ ] HTTP 400 + `model_max_prompt_tokens_exceeded` returns `AppError::PromptTooLong`
- [ ] HTTP 400 with other error codes still returns `AppError::CopilotError`
- [ ] HTTP 429 handling unchanged
- [ ] Other status codes unchanged
- [ ] Info-level log emitted for translated errors (not error-level)

---

### Epic 2: 1M Context Model Activation (Day 1, ~0.5 day)

**Status:** Not Started

**Objective:** Detect Claude Code's `anthropic-beta: context-1m-*` HTTP header and append `-1m` to the normalized Copilot model name, enabling 1M context windows via the Copilot API's distinct model ID (e.g., `claude-opus-4.6-1m`).

#### Task 2.1: Add `has_1m_context_beta()` helper function

**File:** `src/handlers/messages.rs` (MODIFIED)

**Description:** Add a helper function that checks whether the `anthropic-beta` HTTP header contains a `context-1m-*` beta. Uses prefix matching (`context-1m-`) to be forward-compatible with future date suffixes. Handles both comma-separated values in a single header and multiple repeated headers.

**Implementation:**

Add at the top of the file (with the other imports):
```rust
use axum::http::HeaderMap;
```

Add as a module-level function:
```rust
/// Check if the `anthropic-beta` header contains the 1M context beta.
///
/// Claude Code sends beta headers as a comma-separated list:
///   `anthropic-beta: context-1m-2025-08-07,interleaved-thinking-2025-05-14,...`
///
/// Uses prefix matching (`context-1m-`) to be forward-compatible with
/// future date suffixes (the date portion may change across Claude Code versions).
fn has_1m_context_beta(headers: &HeaderMap) -> bool {
    headers
        .get_all("anthropic-beta")
        .iter()
        .any(|value| {
            value.to_str().ok().map_or(false, |s| {
                s.split(',')
                    .any(|beta| beta.trim().starts_with("context-1m-"))
            })
        })
}
```

**Acceptance Criteria:**
- [ ] Returns `true` when `anthropic-beta` contains `context-1m-2025-08-07` (or any `context-1m-*` value)
- [ ] Returns `false` when `anthropic-beta` is absent
- [ ] Returns `false` when `anthropic-beta` contains other betas but not `context-1m-*`
- [ ] Handles comma-separated values in a single header value
- [ ] Handles multiple `anthropic-beta` headers (HTTP allows repeated headers)
- [ ] Handles whitespace around comma-separated values

**Notes:**
- The `anthropic-beta` header value format is `beta1,beta2,beta3` (comma-separated, may have spaces after commas)
- The specific beta value is `context-1m-2025-08-07` as of April 2026, defined in Claude Code's `src/constants/betas.ts:6`
- Prefix matching on `context-1m-` ensures the adapter doesn't need updates when the beta version date changes

#### Task 2.2: Extract `HeaderMap` in messages handler

**File:** `src/handlers/messages.rs` (MODIFIED)

**Description:** Add `axum::http::HeaderMap` extraction to the `messages()` handler signature. axum supports multiple extractors — `HeaderMap` must come before `Json` since `Json` consumes the request body. This is a zero-cost extraction.

**Implementation:**

Update handler signature (~line 38):
```rust
// Before:
pub async fn messages(
    State(state): State<Arc<AppState>>,
    Json(request): Json<AnthropicRequest>,
) -> Result<Response, AppError> {

// After:
pub async fn messages(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(request): Json<AnthropicRequest>,
) -> Result<Response, AppError> {
```

**Acceptance Criteria:**
- [ ] Handler compiles with `HeaderMap` extractor
- [ ] Existing request routing unchanged (axum route registration in `server.rs` doesn't need changes)
- [ ] `Json<AnthropicRequest>` extraction still works (ordering: `HeaderMap` before `Json`)

**Notes:** axum's `HeaderMap` extractor is zero-copy — it passes a reference to the already-parsed header map. No performance impact.

#### Task 2.3: Apply `-1m` suffix based on beta header

**File:** `src/handlers/messages.rs` (MODIFIED)

**Description:** After the existing `to_chat_completion_request()` call (which normalizes the model name via `model_mapper::normalize_model_name()`), conditionally append `-1m` to the model name if the `context-1m-*` beta was detected. This transforms e.g., `claude-opus-4.6` → `claude-opus-4.6-1m`.

**Implementation:**

Add after the `to_chat_completion_request()` call, before sending to the Copilot API. The exact location depends on the streaming vs non-streaming code paths — the suffix must be applied in both. Find all places where `request.to_chat_completion_request(...)` is called and apply the suffix immediately after:

```rust
let wants_1m = has_1m_context_beta(&headers);

let mut chat_request = request.to_chat_completion_request(use_native_tools);

// If Claude Code requested 1M context via the anthropic-beta header,
// select the Copilot API's 1M model variant. The model mapper has already
// normalized the name (e.g., "claude-opus-4-6" → "claude-opus-4.6").
// We append "-1m" to produce "claude-opus-4.6-1m", which is a distinct
// model ID in the Copilot API with ~1M token context window.
if wants_1m && !chat_request.model.contains("-1m") {
    tracing::info!(
        original_model = %chat_request.model,
        "1M context beta detected, selecting Copilot 1M model variant"
    );
    chat_request.model = format!("{}-1m", chat_request.model);
}
```

**Acceptance Criteria:**
- [ ] `claude-opus-4-6` with `context-1m-*` beta → `claude-opus-4.6-1m` sent to Copilot
- [ ] `claude-opus-4-6` without beta → `claude-opus-4.6` sent to Copilot (no change)
- [ ] `claude-opus-4-6-1m` with beta → `claude-opus-4.6-1m` (no double-append, guard `!contains("-1m")`)
- [ ] Applied in both streaming and non-streaming code paths
- [ ] Info-level log emitted when 1M model is selected
- [ ] TRACE-level log (existing) shows the final model name sent to Copilot

**Notes:**
- The guard `!chat_request.model.contains("-1m")` prevents double-appending if someone manually sets a model name with `-1m`
- The suffix is applied **after** `to_chat_completion_request()` (not before) to append to the normalized model name
- Both streaming and non-streaming paths call `to_chat_completion_request()` — both need the suffix applied
- Check the handler code to identify all call sites; there may be separate paths for native tools vs XML tools

---

### Epic 3: Truncated Tool Call Recovery (Day 1-2, ~0.5 day)

**Status:** Not Started

**Objective:** When a tool call is truncated by the output token limit (`finish_reason: "length"`), emit a descriptive text content block instead of silently dropping the incomplete tool_use block. This ensures Claude Code sees a text-only response and can fire max_tokens escalation.

#### Task 3.1: Add `has_emitted_tool_use` tracking field

**File:** `src/streaming/state.rs` (MODIFIED)

**Description:** Add a boolean field to `StreamingState` that tracks whether at least one complete tool_use block has been flushed (emitted to the consumer). This is used to understand the response context when a truncation occurs.

**Implementation:**

Add field to `StreamingState` struct (~line 32-68):
```rust
/// Whether at least one complete tool_use block has been flushed
/// (emitted to the consumer). Used to decide the content of the
/// truncation notice: when `false`, the truncation text is the only
/// content the consumer has seen, maximizing the chance that Claude
/// Code's max_tokens escalation fires.
has_emitted_tool_use: bool,
```

Initialize in `new()`:
```rust
has_emitted_tool_use: false,
```

Set in `flush_tool_use_buffer()` (~lines 406-412):
```rust
fn flush_tool_use_buffer(&mut self) -> Vec<StreamEvent> {
    if self.tool_use_buffer.is_empty() {
        return Vec::new();
    }
    self.has_emitted_tool_use = true;   // NEW
    std::mem::take(&mut self.tool_use_buffer)
}
```

**Acceptance Criteria:**
- [ ] Field initialized to `false` in `new()`
- [ ] Set to `true` when non-empty tool_use buffer is flushed
- [ ] Not affected by empty buffer flushes
- [ ] Existing tests still pass (field is additive)

#### Task 3.2: Emit text block on tool call truncation

**File:** `src/streaming/state.rs` (MODIFIED)

**Description:** Modify the `handle_finish()` method (lines 336-404) to emit a text content block with a truncation notice when `finish_reason: "length"` truncates a tool call, instead of silently dropping the incomplete tool_use block.

**Implementation:**

Replace the truncation path in `handle_finish()` (currently lines ~348-360):

```rust
// Before:
if reason == "length" && self.current_block_type == Some(ContentBlockType::ToolUse) {
    tracing::warn!(/* ... */ "Dropping truncated tool_use block (finish_reason=\"length\")");
    self.tool_use_buffer.clear();
    if let Some(oi_idx) = self.current_openai_tool_index {
        self.truncated_openai_tool_indices.insert(oi_idx);
    }
    self.block_open = false;
    self.current_block_type = None;
}

// After:
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

**Acceptance Criteria:**
- [ ] Truncated tool call emits `ContentBlockStart(text)` + `ContentBlockDelta(notice)` + `ContentBlockStop`
- [ ] Notice text is `[Tool call to "ToolName" was truncated due to output token limit]`
- [ ] Tool name extracted from `tool_call_names` map; defaults to `"unknown"` if not found
- [ ] `current_block_index` incremented after the text block
- [ ] `stop_reason: max_tokens` still emitted after the text block (existing behavior in the code path that follows)
- [ ] Truncation still recorded in `truncated_openai_tool_indices`
- [ ] Incomplete tool_use buffer still cleared
- [ ] `block_open` and `current_block_type` still reset
- [ ] Log message updated with tool name and block index for better diagnostics

**Notes:**
- `ResponseContentBlock::text(String::new())` is the existing constructor (line 302-308 in state.rs) — the empty string is the initial text for the block start; the actual content comes via the delta.
- The text block is emitted *before* the `MessageDelta { stop_reason: "max_tokens" }` that follows in the existing code path. The order is correct: content blocks first, then message_delta.
- The `[square bracket]` format is a system annotation convention, clearly distinguishable from model-generated text.

---

### Epic 4: Testing (Day 2, ~0.75 day)

**Status:** Not Started

**Objective:** Ensure all three fixes are thoroughly tested with unit tests, integration tests, and documented manual E2E test procedures.

#### Task 4.1: Unit Tests — Error Translation

**File:** `tests/unit/error_tests.rs` (MODIFIED)

**Tests to implement:**

1. **`prompt_too_long_returns_400_with_anthropic_format`** — Verify `PromptTooLong` error produces HTTP 400 with correct JSON structure:
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
   ```
   - [ ] Test passes

2. **`prompt_too_long_message_matches_claude_code_regex`** — Simulate the Anthropic SDK's `makeMessage` behavior and verify Claude Code's regex matches:
   ```rust
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

       // Claude Code's regex from src/services/api/errors.ts:89-90
       let re = regex::Regex::new(
           r"(?i)prompt is too long[^0-9]*(\d+)\s*tokens?\s*>\s*(\d+)"
       ).unwrap();
       let caps = re.captures(&sdk_message).expect("regex must match SDK message");
       assert_eq!(caps.get(1).unwrap().as_str(), "168929");
       assert_eq!(caps.get(2).unwrap().as_str(), "168000");
   }
   ```
   - [ ] Test passes

3. **`prompt_too_long_error_type`** — Verify `error_type()` returns `"invalid_request_error"`:
   ```rust
   #[test]
   fn prompt_too_long_error_type() {
       let err = AppError::PromptTooLong {
           actual_tokens: 100000,
           limit_tokens: 50000,
       };
       assert_eq!(err.error_type(), "invalid_request_error");
   }
   ```
   - [ ] Test passes

**Acceptance Criteria:**
- [ ] All 3 new error tests pass
- [ ] Existing error tests still pass

**Notes:** The regex test is the most important — it validates the end-to-end error message chain from adapter through Anthropic SDK to Claude Code's detection logic.

#### Task 4.2: Unit Tests — Copilot Error Parsing

**File:** `tests/unit/copilot_client_tests.rs` (MODIFIED)

**Tests to implement:**

1. **`parse_prompt_too_long_valid_body`** — Standard Copilot error format:
   ```rust
   #[test]
   fn parse_prompt_too_long_valid_body() {
       let body = r#"{"error":{"message":"prompt token count of 168929 exceeds the limit of 168000","code":"model_max_prompt_tokens_exceeded"}}"#;
       assert_eq!(parse_prompt_too_long(body), Some((168929, 168000)));
   }
   ```
   - [ ] Test passes

2. **`parse_prompt_too_long_different_numbers`** — Different token counts:
   ```rust
   #[test]
   fn parse_prompt_too_long_different_numbers() {
       let body = r#"{"error":{"message":"prompt token count of 50000 exceeds the limit of 32000","code":"model_max_prompt_tokens_exceeded"}}"#;
       assert_eq!(parse_prompt_too_long(body), Some((50000, 32000)));
   }
   ```
   - [ ] Test passes

3. **`parse_prompt_too_long_wrong_code`** — Non-matching error code:
   ```rust
   #[test]
   fn parse_prompt_too_long_wrong_code() {
       let body = r#"{"error":{"message":"something else","code":"other_error"}}"#;
       assert_eq!(parse_prompt_too_long(body), None);
   }
   ```
   - [ ] Test passes

4. **`parse_prompt_too_long_invalid_json`** — Invalid JSON body:
   ```rust
   #[test]
   fn parse_prompt_too_long_invalid_json() {
       assert_eq!(parse_prompt_too_long("not json"), None);
   }
   ```
   - [ ] Test passes

5. **`parse_prompt_too_long_missing_message`** — Correct code but no message:
   ```rust
   #[test]
   fn parse_prompt_too_long_missing_message() {
       let body = r#"{"error":{"code":"model_max_prompt_tokens_exceeded"}}"#;
       assert_eq!(parse_prompt_too_long(body), None);
   }
   ```
   - [ ] Test passes

6. **`parse_prompt_too_long_empty_body`** — Empty string:
   ```rust
   #[test]
   fn parse_prompt_too_long_empty_body() {
       assert_eq!(parse_prompt_too_long(""), None);
   }
   ```
   - [ ] Test passes

**Acceptance Criteria:**
- [ ] All 6 new parsing tests pass
- [ ] Existing copilot client tests still pass

#### Task 4.3: Unit Tests — 1M Context Beta Detection

**File:** `tests/unit/messages_tests.rs` (NEW or MODIFIED)

**Description:** Test the `has_1m_context_beta()` helper and the end-to-end model name transformation when the `anthropic-beta` header is present.

**Tests to implement:**

1. **`has_1m_context_beta_present`** — Header contains the beta:
   ```rust
   #[test]
   fn has_1m_context_beta_present() {
       let mut headers = HeaderMap::new();
       headers.insert("anthropic-beta", "context-1m-2025-08-07".parse().unwrap());
       assert!(has_1m_context_beta(&headers));
   }
   ```
   - [ ] Test passes

2. **`has_1m_context_beta_absent`** — Header doesn't contain the beta:
   ```rust
   #[test]
   fn has_1m_context_beta_absent() {
       let mut headers = HeaderMap::new();
       headers.insert("anthropic-beta", "interleaved-thinking-2025-05-14".parse().unwrap());
       assert!(!has_1m_context_beta(&headers));
   }
   ```
   - [ ] Test passes

3. **`has_1m_context_beta_no_header`** — No `anthropic-beta` header at all:
   ```rust
   #[test]
   fn has_1m_context_beta_no_header() {
       let headers = HeaderMap::new();
       assert!(!has_1m_context_beta(&headers));
   }
   ```
   - [ ] Test passes

4. **`has_1m_context_beta_comma_separated`** — Mixed with other betas:
   ```rust
   #[test]
   fn has_1m_context_beta_comma_separated() {
       let mut headers = HeaderMap::new();
       headers.insert(
           "anthropic-beta",
           "interleaved-thinking-2025-05-14,context-1m-2025-08-07".parse().unwrap(),
       );
       assert!(has_1m_context_beta(&headers));
   }
   ```
   - [ ] Test passes

5. **`has_1m_context_beta_future_date`** — Forward-compatible with new date suffix:
   ```rust
   #[test]
   fn has_1m_context_beta_future_date() {
       let mut headers = HeaderMap::new();
       headers.insert("anthropic-beta", "context-1m-2026-12-31".parse().unwrap());
       assert!(has_1m_context_beta(&headers));
   }
   ```
   - [ ] Test passes

6. **`model_name_with_1m_beta_appends_suffix`** — End-to-end model name:
   ```rust
   #[test]
   fn model_name_with_1m_beta_appends_suffix() {
       // Simulate: normalize "claude-opus-4-6" → "claude-opus-4.6", then append "-1m"
       let normalized = normalize_model_name("claude-opus-4-6");
       assert_eq!(normalized, "claude-opus-4.6");
       let with_1m = format!("{}-1m", normalized);
       assert_eq!(with_1m, "claude-opus-4.6-1m");
   }
   ```
   - [ ] Test passes

7. **`model_name_no_double_append`** — Guard prevents double-appending:
   ```rust
   #[test]
   fn model_name_no_double_append() {
       let model = "claude-opus-4.6-1m";
       assert!(model.contains("-1m"));
       // Guard should prevent appending again
   }
   ```
   - [ ] Test passes

**Acceptance Criteria:**
- [ ] All 7 new 1M context tests pass
- [ ] `has_1m_context_beta` is accessible to tests (either `pub(crate)` or tested via the handler)

**Notes:** If `has_1m_context_beta()` is private to the handler module, tests can either: (a) make it `pub(crate)` for test access, or (b) test it indirectly via integration tests (Task 4.6). Prefer (a) for faster feedback.

#### Task 4.4: Unit Tests — Streaming Truncation

**File:** `tests/unit/streaming_tests.rs` (MODIFIED)

**Description:** Update existing truncation tests to expect the new text notice block, and add new tests for edge cases.

**Tests to update:**

1. **`tool_call_truncated_by_length`** (existing, lines 871-899) — Update to expect text block instead of just `message_delta`:
   ```rust
   // Was: assert_eq!(events.len(), 1); assert_message_delta(&events[0], "max_tokens");
   // Now: assert_eq!(events.len(), 4);
   //      assert_text_block_start(&events[0], 0);
   //      assert_text_delta(&events[1], 0, "[Tool call to \"Write\" was truncated ...]");
   //      assert_block_stop(&events[2], 0);
   //      assert_message_delta(&events[3], "max_tokens");
   ```
   - [ ] Test updated and passes

2. **`text_then_tool_truncated_by_length`** (existing, lines 904-938) — Update to expect text notice as a separate block after the original text block:
   ```rust
   // Text block at index 0 emitted normally
   // Truncation text block at index 1
   ```
   - [ ] Test updated and passes

3. **`first_tool_complete_second_truncated`** (existing, lines 943-981) — Update to expect text notice after the first tool_use block:
   ```rust
   // Tool A flushed at index 0
   // Truncation text block at index 1 (for tool B)
   ```
   - [ ] Test updated and passes

4. **`tool_call_with_length_finish_but_complete_json`** (existing, lines 986-1007) — Update to expect text notice (always-drop policy unchanged; notice still emitted):
   - [ ] Test updated and passes

**New tests to add:**

5. **`tool_truncated_unknown_name`** — Tool call with no name in `tool_call_names` map produces `"unknown"` in notice:
   ```rust
   #[test]
   fn tool_truncated_unknown_name() {
       // Construct a scenario where tool name is not recorded
       // Verify notice contains "unknown"
   }
   ```
   - [ ] Test passes

6. **`truncation_notice_block_index_correct_after_text`** — Verify the text notice gets the correct block index when preceded by a text block:
   ```rust
   #[test]
   fn truncation_notice_block_index_correct_after_text() {
       // Text block at index 0
       // Tool call starts (text block closed)
       // Tool truncated → notice at index 1
   }
   ```
   - [ ] Test passes

**Acceptance Criteria:**
- [ ] All 4 existing truncation tests updated and pass
- [ ] 2 new truncation edge-case tests pass
- [ ] All other existing streaming tests still pass (non-truncation paths unchanged)

#### Task 4.5: Integration Tests — Prompt-Too-Long Translation

**File:** `tests/integration/error_tests.rs` (MODIFIED)

**Description:** Add an integration test that sends a request through the full adapter with a mock Copilot API returning 400 `model_max_prompt_tokens_exceeded`, and verifies the adapter returns HTTP 400 with the correct Anthropic-format error.

**Scenario:**
1. **Setup:** Spawn a mock Copilot API that returns HTTP 400 with `model_max_prompt_tokens_exceeded` body for any `/chat/completions` request. Use the existing `spawn_mock_github()` and `create_test_state()` patterns.
2. **Action:** Send `POST /v1/messages` with a valid Anthropic-format request through the adapter router.
3. **Verification:**
   - Response status is 400
   - Response body is `{"error": {"message": "prompt is too long: N tokens > M maximum", "type": "invalid_request_error", "code": "prompt_too_long"}}`
   - The message matches Claude Code's regex

```rust
#[tokio::test]
async fn copilot_prompt_too_long_translated_to_anthropic_format() {
    // Spawn mock Copilot returning 400 model_max_prompt_tokens_exceeded
    let (copilot_addr, _h) = spawn_mock_copilot_prompt_too_long().await;
    // ... create test state, build router ...
    let response = app.oneshot(/* POST /v1/messages */).await.unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    // ... parse body, verify message matches regex ...
}
```

- [ ] Test passes

**Helper to add:**
```rust
async fn spawn_mock_copilot_prompt_too_long() -> (SocketAddr, JoinHandle<()>) {
    // Returns HTTP 400 with:
    // {"error":{"message":"prompt token count of 168929 exceeds the limit of 168000",
    //           "code":"model_max_prompt_tokens_exceeded"}}
}
```

**Acceptance Criteria:**
- [ ] Integration test passes
- [ ] Existing integration error tests still pass

#### Task 4.6: Integration Tests — Streaming Truncation

**File:** `tests/integration/streaming_tests.rs` (NEW or MODIFIED — check if file exists)

**Description:** Add an integration test that sends a streaming request through the adapter with a mock Copilot API that returns SSE chunks ending with `finish_reason: "length"` mid-tool-call, and verifies the SSE stream contains the truncation notice text block.

**Scenario:**
1. **Setup:** Spawn a mock Copilot API that returns a streaming SSE response with tool call chunks followed by `finish_reason: "length"`.
2. **Action:** Send `POST /v1/messages` with `stream: true` and tool definitions.
3. **Verification:**
   - SSE stream contains `content_block_start` with `type: "text"`
   - SSE stream contains `content_block_delta` with the truncation notice
   - SSE stream contains `message_delta` with `stop_reason: "max_tokens"`
   - SSE stream does NOT contain any `type: "tool_use"` content blocks

- [ ] Test passes

**Acceptance Criteria:**
- [ ] SSE stream validated end-to-end
- [ ] No tool_use blocks in output

#### Task 4.7: Integration Tests — 1M Context Model Selection

**File:** `tests/integration/messages_tests.rs` (MODIFIED)

**Description:** Add integration tests that send requests through the adapter with and without the `anthropic-beta: context-1m-*` header, verifying the correct model name is forwarded to the Copilot API.

**Scenario 1: 1M context activated**
1. **Setup:** Spawn a mock Copilot API that captures the request body.
2. **Action:** Send `POST /v1/messages` with `anthropic-beta: context-1m-2025-08-07` header and `model: "claude-opus-4-6"`.
3. **Verification:**
   - The mock Copilot receives `model: "claude-opus-4.6-1m"` in the request body

**Scenario 2: Standard context (no beta)**
1. **Setup:** Same mock.
2. **Action:** Send `POST /v1/messages` without the `context-1m-*` beta and `model: "claude-opus-4-6"`.
3. **Verification:**
   - The mock Copilot receives `model: "claude-opus-4.6"` (no `-1m` suffix)

- [ ] Both scenarios pass

**Acceptance Criteria:**
- [ ] 1M beta header → model name has `-1m` suffix
- [ ] No beta header → model name unchanged
- [ ] Existing integration message tests still pass

#### Task 4.8: Manual E2E Test Procedures

**File:** `docs/development/e2e-testing.md` (MODIFIED)

**Test procedures to add:**

1. **Prompt-too-long recovery:**
   ```
   1. Start copilot-adapter with --log-level debug
   2. Start a Claude Code session with a long conversation
   3. Continue until the prompt approaches 168K tokens
   4. Observe that Claude Code receives "prompt too long" error
   5. Verify Claude Code triggers context compaction (visible in Claude Code output)
   6. Verify the adapter logs show "Translating prompt-too-long error to Anthropic format"
   ```
   - Expected: Claude Code compacts context and continues the session
   - [ ] Documented

2. **Truncated tool call escalation:**
   ```
   1. Start copilot-adapter with --log-level debug
   2. Start a Claude Code session
   3. Ask Claude to write a very large file (>8K tokens of content)
   4. Observe that the first attempt uses default max_tokens (8K)
   5. Observe that the tool call is truncated (adapter log: "Dropping truncated tool_use block")
   6. Observe that Claude Code escalates max_tokens and retries
   7. Verify the second attempt with 64K budget succeeds
   ```
   - Expected: File write succeeds on retry with escalated token budget
   - [ ] Documented

3. **1M context model activation:**
   ```
   1. Start copilot-adapter with --log-level debug
   2. Start Claude Code and select "Opus (1M context)" from the model picker
   3. Send a message and observe the adapter logs
   4. Verify the adapter log shows "1M context beta detected, selecting Copilot 1M model variant"
   5. Verify the adapter log shows model="claude-opus-4.6-1m" in the outgoing request
   6. Verify the conversation works normally with the 1M model
   7. Optionally: start a long conversation and verify it doesn't hit the 168K limit
   ```
   - Expected: Adapter forwards requests to `claude-opus-4.6-1m`; longer conversations are supported
   - [ ] Documented

**Acceptance Criteria:**
- [ ] Both E2E test procedures documented
- [ ] Steps are reproducible

---

### Epic 5: Documentation (Day 2-3, ~0.25 day)

**Status:** Not Started

**Objective:** Update project documentation to reflect the new error handling and streaming behavior.

#### Task 5.1: Update CLAUDE.md

**File:** `CLAUDE.md` (MODIFIED)

**Changes:**
- Add a note under "Notes for Development" about prompt-too-long error translation:
  - `PromptTooLong` error variant, when it's emitted, how it maps to HTTP 400
  - The critical format string requirement for Claude Code's regex
- Update the existing streaming/truncation note to mention the text notice block behavior
- Add a note about 1M context model activation:
  - The `anthropic-beta: context-1m-*` header detection mechanism
  - How `-1m` is appended to the Copilot model name
  - The relationship between Claude Code's `[1m]` suffix, the beta header, and the Copilot model ID

**Acceptance Criteria:**
- [ ] CLAUDE.md updated with prompt-too-long error handling info
- [ ] CLAUDE.md updated with truncation notice behavior
- [ ] CLAUDE.md updated with 1M context model activation info

#### Task 5.2: Update known issues

**File:** `docs/known-issues.md` (MODIFIED)

**Changes:**
- If the prompt-too-long and truncation issues are listed as known issues, mark them as resolved with a reference to this implementation
- If not listed, add them as resolved items for historical reference

**Acceptance Criteria:**
- [ ] Known issues document updated

#### Task 5.3: Archive design document

**File:** `docs/design/CONTEXT-WINDOW-AND-TRUNCATION.design.md` (MODIFIED)

**Changes:**
- Update status from "Draft" to "Implemented"
- Update open questions if any were resolved during implementation

**Acceptance Criteria:**
- [ ] Design document status updated

---

## Requirements

### Functional Requirements

| ID | Requirement | Source | Epic |
|----|-------------|--------|------|
| FR1 | Copilot API 400 `model_max_prompt_tokens_exceeded` → adapter HTTP 400 `invalid_request_error` | Design doc §Option A | Epic 1 |
| FR2 | Error message matches Claude Code regex `/prompt is too long[^0-9]*(\d+)\s*tokens?\s*>\s*(\d+)/i` | Design doc §Research KF2 | Epic 1 |
| FR3 | Token counts extracted from Copilot error and reformatted in Anthropic style | Design doc §Option A.1 | Epic 1 |
| FR4 | `anthropic-beta: context-1m-*` header → model name appended with `-1m` | Design doc §Option C | Epic 2 |
| FR5 | Without `context-1m-*` beta, model name unchanged | Design doc §Option C | Epic 2 |
| FR6 | No double-append when model name already contains `-1m` | Design doc §Option C.6 | Epic 2 |
| FR7 | Truncated tool call emits text block with `[Tool call to "X" was truncated due to output token limit]` | Design doc §Option E.2 | Epic 3 |
| FR8 | `stop_reason: max_tokens` preserved after truncation notice | Design doc §Option E.2 | Epic 3 |
| FR9 | Unrecognized Copilot 400 errors still return HTTP 502 `upstream_error` (existing behavior) | Design doc §NG3, NG4 | Epic 1 |

### Non-Functional Requirements

| ID | Requirement | Target | Epic |
|----|-------------|--------|------|
| NFR1 | No new crate dependencies | Zero new dependencies | All |
| NFR2 | Error translation adds negligible latency | <1ms additional parsing | Epic 1 |
| NFR3 | No changes to non-error streaming paths | Identical SSE output for successful requests | Epic 3 |
| NFR4 | Beta header detection is forward-compatible | Prefix match `context-1m-` works for future dates | Epic 2 |

---

## File Changes Summary

| File | Change | Epic | Description |
|------|--------|------|-------------|
| `src/error.rs` | Modified | Epic 1 | Add `PromptTooLong` variant, HTTP 400 mapping, `error_type()` arm |
| `src/copilot/client.rs` | Modified | Epic 1 | Add `parse_prompt_too_long()` function, update `handle_error_response()` |
| `src/handlers/messages.rs` | Modified | Epic 2 | Add `HeaderMap` extraction, `has_1m_context_beta()`, append `-1m` to model name |
| `src/streaming/state.rs` | Modified | Epic 3 | Add `has_emitted_tool_use` field, emit text block on truncation |
| `tests/unit/error_tests.rs` | Modified | Epic 4 | Add 3 tests: HTTP response format, regex match, error_type |
| `tests/unit/copilot_client_tests.rs` | Modified | Epic 4 | Add 6 tests: prompt-too-long parsing (valid, edge cases, failures) |
| `tests/unit/messages_tests.rs` | New/Modified | Epic 4 | Add 7 tests: 1M beta detection, model name transformation |
| `tests/unit/streaming_tests.rs` | Modified | Epic 4 | Update 4 existing truncation tests, add 2 new edge-case tests |
| `tests/integration/error_tests.rs` | Modified | Epic 4 | Add 1 integration test: prompt-too-long end-to-end |
| `tests/integration/messages_tests.rs` | Modified | Epic 4 | Add 2 integration tests: 1M model selection with/without beta |
| `tests/integration/streaming_tests.rs` | Modified/New | Epic 4 | Add 1 integration test: truncation notice in SSE stream |
| `docs/development/e2e-testing.md` | Modified | Epic 4 | Add 3 manual E2E test procedures |
| `CLAUDE.md` | Modified | Epic 5 | Add development notes for prompt-too-long, 1M context, and truncation handling |
| `docs/known-issues.md` | Modified | Epic 5 | Update/add resolved issue entries |
| `docs/design/CONTEXT-WINDOW-AND-TRUNCATION.design.md` | Modified | Epic 5 | Update status to "Implemented" |

---

## Testing Strategy

### Test Coverage

| Component | Unit Tests | Integration Tests | E2E Tests |
|-----------|------------|-------------------|-----------|
| `PromptTooLong` error variant | Task 4.1 (3 tests) | Task 4.5 (1 test) | Task 4.8 (procedure 1) |
| `parse_prompt_too_long()` parser | Task 4.2 (6 tests) | Task 4.5 (1 test) | — |
| `handle_error_response()` 400 path | — | Task 4.5 (1 test) | Task 4.8 (procedure 1) |
| `has_1m_context_beta()` detection | Task 4.3 (7 tests) | Task 4.7 (2 tests) | Task 4.8 (procedure 3) |
| Model name `-1m` appending | Task 4.3 (2 tests) | Task 4.7 (2 tests) | Task 4.8 (procedure 3) |
| Truncation text notice | Task 4.4 (6 tests) | Task 4.6 (1 test) | Task 4.8 (procedure 2) |
| `has_emitted_tool_use` tracking | Task 4.4 (implicit) | — | — |

### Test Files

| File | Type | Coverage |
|------|------|----------|
| `tests/unit/error_tests.rs` | Unit | PromptTooLong error format + regex match |
| `tests/unit/copilot_client_tests.rs` | Unit | Error body parsing (6 cases) |
| `tests/unit/messages_tests.rs` | Unit | 1M beta detection + model name transformation (7 tests) |
| `tests/unit/streaming_tests.rs` | Unit | Truncation notice emission (6 tests) |
| `tests/integration/error_tests.rs` | Integration | Full HTTP round-trip for prompt-too-long |
| `tests/integration/messages_tests.rs` | Integration | 1M model selection (2 scenarios) |
| `tests/integration/streaming_tests.rs` | Integration | Full SSE stream with truncation |
| `docs/development/e2e-testing.md` | Manual E2E | Long conversation + large file write + 1M context |

---

## Dependencies

### External Dependencies

| Dependency | Version | Purpose | Epic |
|------------|---------|---------|------|
| None | — | No new dependencies required | — |

**Cargo.toml changes:** None. `regex`, `serde_json`, `thiserror`, and `axum` (including `axum::http::HeaderMap`) are all existing dependencies. The string-parsing approach in `parse_prompt_too_long()` avoids needing `regex` in the production code (only used in tests for the Claude Code regex validation).

### Internal Dependencies

| Module | Required By | Status |
|--------|-------------|--------|
| `src/error.rs` (`AppError`) | Epic 1 | ✅ Exists — adding variant |
| `src/copilot/client.rs` (`CopilotClient`) | Epic 1 | ✅ Exists — adding function + modifying method |
| `src/handlers/messages.rs` (`messages()`) | Epic 2 | ✅ Exists — adding header extraction + helper function |
| `axum::http::HeaderMap` | Epic 2 | ✅ Exists — already a dependency via axum |
| `src/streaming/state.rs` (`StreamingState`) | Epic 3 | ✅ Exists — adding field + modifying method |
| `src/streaming/state.rs` (`ResponseContentBlock::text()`) | Epic 3 | ✅ Exists — constructor already defined |
| `src/streaming/state.rs` (`StreamEvent` variants) | Epic 3 | ✅ Exists — using existing event types |

---

## Risk Assessment

| Risk | Impact | Probability | Mitigation | Epic |
|------|--------|-------------|------------|------|
| Anthropic SDK doesn't match `prompt is too long` in JSON-stringified body | High | Low | Task 4.1 test 2 validates exact SDK behavior with regex | Epic 1 |
| Copilot API changes error message format | Medium | Low | `parse_prompt_too_long` returns `None` → falls through to generic `CopilotError` | Epic 1 |
| Copilot API doesn't have 1M variant for a model Claude Code requests | Medium | Medium | API returns a clear error for unknown model IDs; no silent downgrade | Epic 2 |
| Beta header date format changes | Low | Low | Prefix matching on `context-1m-` is forward-compatible | Epic 2 |
| Non-Claude model gets `-1m` appended | Low | Very Low | Claude Code only sends `context-1m-*` for Claude models; Copilot rejects unknown IDs | Epic 2 |
| Truncation text block confuses the model on retry | Low | Medium | Square-bracket format is clearly system-level; model has seen similar patterns | Epic 3 |
| Updated streaming tests break other streaming tests | Medium | Low | Additive changes only; existing event sequences extended, not replaced | Epic 4 |
| `has_emitted_tool_use` tracking has off-by-one or state leak | Low | Low | Simple boolean, set in one place, tested explicitly | Epic 3 |
| Copilot API 400 errors from other causes wrongly matched as prompt-too-long | Low | Very Low | Checks `code` field specifically, not message text | Epic 1 |

---

## Success Criteria

1. **Prompt too long** — Claude Code's `isPromptTooLongMessage()` returns `true` for the translated error (Epic 1, validated by Task 4.1 test 2)
2. **Token parsing** — `parsePromptTooLongTokenCounts()` extracts correct `actualTokens` and `limitTokens` (Epic 1, validated by Task 4.1 test 2)
3. **1M context activation** — Selecting "Opus (1M context)" in Claude Code results in `claude-opus-4.6-1m` being sent to Copilot (Epic 2, validated by Task 4.7)
4. **Tool truncation** — Text notice block emitted; `stop_reason: max_tokens` preserved (Epic 3, validated by Task 4.4)
5. **No regressions** — All existing streaming and error tests pass (Epic 4)
6. **All new tests passing** — 27 new/updated tests pass (Epic 4)
7. **Documentation complete** — CLAUDE.md, known-issues, and e2e-testing docs updated (Epic 5)

---

## Rollout / Migration Plan

### Phase 1: Development (Epics 1-3)
- [ ] Implement `PromptTooLong` error variant and HTTP mapping (Task 1.1)
- [ ] Implement `parse_prompt_too_long()` parser (Task 1.2)
- [ ] Update `handle_error_response()` (Task 1.3)
- [ ] Add `has_1m_context_beta()` helper (Task 2.1)
- [ ] Extract `HeaderMap` in messages handler (Task 2.2)
- [ ] Apply `-1m` suffix based on beta header (Task 2.3)
- [ ] Add `has_emitted_tool_use` field (Task 3.1)
- [ ] Implement truncation text block emission (Task 3.2)
- [ ] Verify `cargo build` succeeds

### Phase 2: Testing (Epic 4)
- [ ] Unit tests for error translation (Task 4.1)
- [ ] Unit tests for error parsing (Task 4.2)
- [ ] Unit tests for 1M context beta detection (Task 4.3)
- [ ] Unit tests for streaming truncation (Task 4.4)
- [ ] Integration test for prompt-too-long (Task 4.5)
- [ ] Integration test for streaming truncation (Task 4.6)
- [ ] Integration test for 1M model selection (Task 4.7)
- [ ] Manual E2E procedures documented (Task 4.8)
- [ ] `cargo test --test unit` passes
- [ ] `cargo test --test integration` passes

### Phase 3: Documentation (Epic 5)
- [ ] CLAUDE.md updated (Task 5.1)
- [ ] Known issues updated (Task 5.2)
- [ ] Design document status updated (Task 5.3)

### Phase 4: Release
- [ ] All acceptance criteria met
- [ ] Final review
- [ ] Merge to main
- [ ] Move design/plan docs to `docs/design/archive/`

---

## Epic Status Tracking

| Epic | Status | Start Date | End Date | Notes |
|------|--------|------------|----------|-------|
| Epic 1: Prompt-Too-Long Error Translation | Not Started | - | - | 3 tasks |
| Epic 2: 1M Context Model Activation | Not Started | - | - | 3 tasks |
| Epic 3: Truncated Tool Call Recovery | Not Started | - | - | 2 tasks |
| Epic 4: Testing | Not Started | - | - | 8 tasks, 27 tests |
| Epic 5: Documentation | Not Started | - | - | 3 tasks |

---

## Open Questions

| # | Question | Status | Blocker For |
|---|----------|--------|-------------|
| 1 | What is the per-model prompt token limit on the Copilot API? Is 168K the same for all Claude models? | Partially answered — observed 168K for `claude-opus-4.6`; `claude-opus-4.6-1m` presumably accepts ~1M | None — parser extracts whatever numbers Copilot returns |
| 2 | Does `tests/integration/streaming_tests.rs` already exist or does it need to be created? | Resolve at implementation time | Task 4.6 |
| 3 | Should `parse_prompt_too_long` live in `client.rs` or a separate `error_parser.rs` module? | Deferred — start in `client.rs`, refactor if needed | None |
| 4 | Will `claude-sonnet-4.6-1m` appear in the Copilot models list in the future? | Open — currently only `claude-opus-4.6-1m` exists | None — adapter will support it automatically |
| 5 | Does the handler have separate code paths for streaming/non-streaming that both need the `-1m` suffix? | Resolve at implementation time — check all `to_chat_completion_request()` call sites | Task 2.3 |

---

## References

- [Design document](./CONTEXT-WINDOW-AND-TRUNCATION.design.md)
- [Large file write bug research](./LARGE-FILE-WRITE-BUG-RESEARCH.md)
- [Error investigation report](./ERROR_INVESTIGATION_REPORT.md)
- `src/error.rs` — AppError enum and HTTP status mapping
- `src/copilot/client.rs` — `handle_error_response()` (lines 91-112)
- `src/streaming/state.rs` — `handle_finish()` (lines 336-404), `StreamingState` struct (lines 32-68)
- `tests/unit/error_tests.rs` — Existing error format tests with `error_to_parts()` helper
- `tests/unit/streaming_tests.rs` — Existing truncation tests and assertion helpers
- `tests/unit/copilot_client_tests.rs` — Existing client tests with mock patterns
- `tests/integration/error_tests.rs` — Existing integration patterns with `create_test_state()`

---

## Notes

### Implementation Order

The recommended implementation order within each day:

**Day 1:**
1. Task 1.1 (error variant) → Task 1.2 (parser) → Task 1.3 (handle_error_response) — builds bottom-up
2. Task 2.1 (beta helper) → Task 2.2 (header extraction) → Task 2.3 (suffix appending) — builds bottom-up
3. Task 3.1 (tracking field) → Task 3.2 (truncation text block) — builds bottom-up
4. `cargo build` to verify compilation

**Day 2:**
1. Task 4.2 (parser tests — simplest) → Task 4.1 (error tests) → Task 4.3 (1M context tests) → Task 4.4 (streaming tests — most complex)
2. Task 4.5 (integration error test) → Task 4.7 (integration 1M test) → Task 4.6 (integration streaming test)
3. Task 4.8 (E2E docs) → Tasks 5.1-5.3 (documentation)
4. Full test suite: `cargo test`

### Key Invariants

- The error message string `"prompt is too long: N tokens > M maximum"` must NEVER be changed without verifying it matches Claude Code's regex
- The truncation notice format `[Tool call to "X" was truncated due to output token limit]` should use square brackets to distinguish from model text
- `parse_prompt_too_long()` must return `None` (not panic) for any unexpected input
- Existing `CopilotError` behavior must be preserved for all non-matching 400 errors
- `has_1m_context_beta()` must use prefix matching (`context-1m-`) not exact string matching, to be forward-compatible with future beta date suffixes
- The `-1m` suffix must be appended AFTER `normalize_model_name()`, not before, to ensure correct Copilot model ID format
- The guard `!model.contains("-1m")` must prevent double-appending in all code paths

### Development Notes
- [Notes added during implementation]

### Review Notes
- [Code review feedback]

### Testing Notes
- [Test failures and fixes]
- [Edge cases discovered]
