# Context Window Enforcement & Truncated Tool Recovery — Implementation Plan

**Status:** In Progress
**Date:** 2026-04-05
**Based on:** [CONTEXT-WINDOW-AND-TRUNCATION.design.md](./CONTEXT-WINDOW-AND-TRUNCATION.design.md)
**Prerequisite:** None
**Estimated Time:** 2-3 days

---

## Executive Summary

The copilot-adapter has four related issues that cause Claude Code sessions to fail or underperform during long conversations, large file writes, 1M context usage, or when effort/thinking parameters are configured. This plan implements four targeted fixes designed in the companion design document:

1. **Option A — Prompt-too-long error translation:** Translate GitHub Copilot's `model_max_prompt_tokens_exceeded` HTTP 400 error into an Anthropic-format `invalid_request_error` with a message matching Claude Code's prompt-too-long regex, so Claude Code triggers automatic context compaction.
2. **Option C — 1M context model activation:** Detect Claude Code's `anthropic-beta: context-1m-*` HTTP header and append `-1m` to the normalized Copilot model name (e.g., `claude-opus-4.6` → `claude-opus-4.6-1m`), enabling true 1M context windows via the Copilot API's distinct model ID.
3. **Option D — Effort and thinking support:** Accept `output_config.effort` and `thinking` parameters from Claude Code, translate effort to `reasoning.effort` in the OpenAI request, handle `thinking`/`redacted_thinking` content blocks in conversation history (strip during translation), and suppress temperature when thinking is active.
4. **Option E — Truncated tool call recovery:** When a tool call is truncated by `finish_reason: "length"`, emit a descriptive text content block instead of silently dropping the incomplete tool_use block, so Claude Code sees a text-only response and can fire its max_tokens escalation logic (8K → 64K retry).

This plan implements:
- New `PromptTooLong` error variant with Anthropic-compatible HTTP 400 response
- Copilot API error body parser for `model_max_prompt_tokens_exceeded`
- `anthropic-beta` header extraction and `context-1m-*` detection in the messages handler
- Model name `-1m` suffix appending for 1M context activation
- `OutputConfig` struct and `output_config` / `thinking` fields on `AnthropicRequest`
- `Thinking` and `RedactedThinking` content block variants for conversation history
- `Reasoning` struct and effort translation in `to_chat_completion_request()`
- Temperature suppression when thinking is active
- Streaming state machine changes to emit truncation notice text blocks
- New `has_emitted_tool_use` tracking field in `StreamingState`
- Comprehensive unit, integration, and manual E2E tests

**Total estimated time:** 2-3 days

---

## Background

### Current State

- **Error handling (`src/copilot/client.rs`, lines 91-112):** All non-429 Copilot API errors become `AppError::CopilotError` → HTTP 502 `upstream_error`. No special handling for HTTP 400 or `model_max_prompt_tokens_exceeded`.
- **Streaming truncation (`src/streaming/state.rs`, lines 336-404):** When `finish_reason == "length"` mid-tool-call, the adapter clears the `tool_use_buffer`, records the truncated index, and emits only `MessageDelta { stop_reason: "max_tokens" }` with no content blocks.
- **Error types (`src/error.rs`, lines 13-37):** 8 existing variants: `NotAuthenticated`, `TokenExpired`, `GitHubError`, `CopilotError`, `RateLimited`, `InvalidRequest`, `ModelNotFound`, `Internal`.
- **`StreamingState` struct (`src/streaming/state.rs`, lines 32-68):** 13 fields. No tracking of whether complete tool_use blocks have been emitted.
- **`regex` crate:** Already a direct dependency in `Cargo.toml` (line 27: `regex = "1"`).
- **Messages handler (`src/handlers/messages.rs`, line 38-41):** Extracts only `State` and `Json<AnthropicRequest>`. Does not extract HTTP headers. No access to `anthropic-beta` or any other header.
- **`AnthropicRequest` struct (`src/anthropic/types.rs`, lines 228-252):** No `betas` field. The Anthropic SDK sends betas as the `anthropic-beta` HTTP header, not in the JSON body. No `output_config` or `thinking` fields — these are silently discarded during deserialization.
- **`ContentBlock` enum (`src/anthropic/types.rs`, lines 93-132):** No `Thinking` or `RedactedThinking` variants. Requests with these content block types in conversation history cause serde deserialization failures.
- **`ChatCompletionRequest` struct (`src/copilot/types.rs`, lines 104-133):** No `reasoning` field for OpenAI reasoning effort.
- **`model_mapper.rs` (lines 18-85):** Has context marker preservation logic for `-1m`/`-200k` in model names, but this code is unreachable from Claude Code — Claude Code strips `[1m]` before sending and uses the beta header instead.
- **Copilot API models:** Live query to `GET https://api.githubcopilot.com/models` confirms `claude-opus-4.6-1m` exists as a distinct model ID alongside `claude-opus-4.6`.

### Target State

- Copilot API 400 `model_max_prompt_tokens_exceeded` → adapter returns HTTP 400 with `"type": "invalid_request_error"` and message `"prompt is too long: N tokens > M maximum"` → Claude Code triggers context compaction.
- `anthropic-beta: context-1m-*` header → adapter appends `-1m` to normalized model name → Copilot API receives 1M model ID (e.g., `claude-opus-4.6-1m`) → 1M context window activated.
- `output_config.effort` → adapter translates to `reasoning.effort` in the OpenAI request → Copilot API receives effort preference.
- `thinking` / `redacted_thinking` content blocks in conversation history → adapter accepts and strips during translation → no deserialization errors.
- `thinking` parameter present → adapter suppresses temperature forwarding → compatible with thinking-enabled models.
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
| G6 | Effort level is forwarded to the Copilot API | `output_config.effort` → `reasoning.effort` in OpenAI request |
| G7 | Thinking content blocks in history don't break deserialization | `thinking` and `redacted_thinking` content blocks accepted and stripped |

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

**Status:** Complete

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
- [x] `PromptTooLong` variant compiles with correct `#[error]` format
- [x] `IntoResponse` returns HTTP 400 with `"type": "invalid_request_error"`
- [x] `error_type()` returns `"invalid_request_error"`
- [x] Error message exactly matches: `"prompt is too long: {actual} tokens > {limit} maximum"`

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
- [x] Parses `(168929, 168000)` from the standard Copilot error format
- [x] Returns `None` for wrong `code` field
- [x] Returns `None` for invalid JSON
- [x] Returns `None` for missing fields
- [x] Returns `None` for unparseable message text
- [x] Function is `pub` so unit tests can access it

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
- [x] HTTP 400 + `model_max_prompt_tokens_exceeded` returns `AppError::PromptTooLong`
- [x] HTTP 400 with other error codes still returns `AppError::CopilotError`
- [x] HTTP 429 handling unchanged
- [x] Other status codes unchanged
- [x] Info-level log emitted for translated errors (not error-level)

---

### Epic 2: 1M Context Model Activation (Day 1, ~0.5 day)

**Status:** Complete

#### Task 2.1: Add `has_1m_context_beta()` helper function

**File:** `src/handlers/messages.rs` (MODIFIED)

**Description:** Add a helper function that checks whether the `anthropic-beta` HTTP header contains a `context-1m-*` beta. Uses prefix matching (`context-1m`) to be forward-compatible with future date suffixes. Handles both comma-separated values in a single header and multiple repeated headers.

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
/// Uses prefix matching (`context-1m`) to be forward-compatible with
/// future date suffixes (the date portion may change across Claude Code versions).
fn has_1m_context_beta(headers: &HeaderMap) -> bool {
    headers
        .get_all("anthropic-beta")
        .iter()
        .any(|value| {
            value.to_str().ok().map_or(false, |s| {
                s.split(',')
                    .any(|beta| beta.trim().starts_with("context-1m"))
            })
        })
}
```

**Acceptance Criteria:**
- [x] Returns `true` when `anthropic-beta` contains `context-1m-2025-08-07` (or any `context-1m-*` value)
- [x] Returns `false` when `anthropic-beta` is absent
- [x] Returns `false` when `anthropic-beta` contains other betas but not `context-1m-*`
- [x] Handles comma-separated values in a single header value
- [x] Handles multiple `anthropic-beta` headers (HTTP allows repeated headers)
- [x] Handles whitespace around comma-separated values

**Notes:**
- The `anthropic-beta` header value format is `beta1,beta2,beta3` (comma-separated, may have spaces after commas)
- The specific beta value is `context-1m-2025-08-07` as of April 2026, defined in Claude Code's `src/constants/betas.ts:6`
- Prefix matching on `context-1m` ensures the adapter doesn't need updates when the beta version date changes

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
- [x] Handler compiles with `HeaderMap` extractor
- [x] Existing request routing unchanged (axum route registration in `server.rs` doesn't need changes)
- [x] `Json<AnthropicRequest>` extraction still works (ordering: `HeaderMap` before `Json`)

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
- [x] `claude-opus-4-6` with `context-1m-*` beta → `claude-opus-4.6-1m` sent to Copilot
- [x] `claude-opus-4-6` without beta → `claude-opus-4.6` sent to Copilot (no change)
- [x] `claude-opus-4-6-1m` with beta → `claude-opus-4.6-1m` (no double-append, guard `!contains("-1m")`)
- [x] Applied in both streaming and non-streaming code paths
- [x] Info-level log emitted when 1M model is selected
- [x] TRACE-level log (existing) shows the final model name sent to Copilot

**Notes:**
- The guard `!chat_request.model.contains("-1m")` prevents double-appending if someone manually sets a model name with `-1m`
- The suffix is applied **after** `to_chat_completion_request()` (not before) to append to the normalized model name
- Both streaming and non-streaming paths call `to_chat_completion_request()` — both need the suffix applied
- Check the handler code to identify all call sites; there may be separate paths for native tools vs XML tools

---

### Epic 3: Truncated Tool Call Recovery (Day 1-2, ~0.5 day)

**Status:** Complete

**Note:** Task 3.1 (`has_emitted_tool_use` field) was removed during review — the field was a dead-write (set but never read). The truncation notice behavior in Task 3.2 is uniform regardless of prior tool_use blocks, so the field added unnecessary complexity. Task 3.2 is implemented as specified.

#### Task 3.1: ~~Add `has_emitted_tool_use` tracking field~~ (Removed)

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
- [x] Truncated tool call emits `ContentBlockStart(text)` + `ContentBlockDelta(notice)` + `ContentBlockStop`
- [x] Notice text is `[Tool call to "ToolName" was truncated due to output token limit]`
- [x] Tool name extracted from `tool_call_names` map; defaults to `"unknown"` if not found
- [x] `current_block_index` incremented after the text block
- [x] `stop_reason: max_tokens` still emitted after the text block (existing behavior in the code path that follows)
- [x] Truncation still recorded in `truncated_openai_tool_indices`
- [x] Incomplete tool_use buffer still cleared
- [x] `block_open` and `current_block_type` still reset
- [x] Log message updated with tool name and block index for better diagnostics

**Notes:**
- `ResponseContentBlock::text(String::new())` is the existing constructor (line 302-308 in state.rs) — the empty string is the initial text for the block start; the actual content comes via the delta.
- The text block is emitted *before* the `MessageDelta { stop_reason: "max_tokens" }` that follows in the existing code path. The order is correct: content blocks first, then message_delta.
- The `[square bracket]` format is a system annotation convention, clearly distinguishable from model-generated text.

---

### Epic 4: Effort and Thinking Support (Day 2, ~0.5 day)

**Status:** Not Started

**Objective:** Accept `output_config.effort` and `thinking` parameters from Claude Code, translate effort to `reasoning.effort` in the OpenAI request, handle `thinking`/`redacted_thinking` content blocks in conversation history, and suppress temperature when thinking is active.

#### Task 4.1: Add `OutputConfig` struct and fields to `AnthropicRequest`

**File:** `src/anthropic/types.rs` (MODIFIED)

**Description:** Add an `OutputConfig` struct with an `effort` field, and add `output_config` and `thinking` fields to `AnthropicRequest`. The `thinking` field uses `serde_json::Value` for forward-compatibility.

**Implementation:**

Add the `OutputConfig` struct (before `AnthropicRequest`):
```rust
/// Anthropic output configuration.
///
/// Currently only `effort` is used by the adapter. Other fields (`format`,
/// `task_budget`) are accepted via serde's default behavior and not forwarded.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OutputConfig {
    /// Effort level: "low", "medium", "high", or "max".
    #[serde(skip_serializing_if = "Option::is_none")]
    pub effort: Option<String>,
}
```

Add fields to `AnthropicRequest` (after `tool_choice`):
```rust
/// Output configuration including effort level.
///
/// Claude Code sends `output_config.effort` to control model reasoning depth.
/// Translated to `reasoning.effort` in the OpenAI request.
#[serde(skip_serializing_if = "Option::is_none")]
pub output_config: Option<OutputConfig>,

/// Thinking configuration.
///
/// The adapter notes its presence (to suppress temperature forwarding) but
/// does not translate it to OpenAI format — effort level is the closest
/// approximation.
#[serde(skip_serializing_if = "Option::is_none")]
pub thinking: Option<serde_json::Value>,
```

**Acceptance Criteria:**
- [ ] `AnthropicRequest` with `output_config` and `thinking` deserializes correctly
- [ ] `AnthropicRequest` without these fields still deserializes (backward compatible)
- [ ] `OutputConfig` captures `effort` field
- [ ] Extra fields in `output_config` (e.g., `format`, `task_budget`) are silently ignored
- [ ] `thinking` captures any JSON value shape

**Notes:** `thinking` is `Option<serde_json::Value>` rather than a typed struct because the adapter only needs to detect its presence (for temperature suppression), not interpret its structure.

#### Task 4.2: Add `Thinking` and `RedactedThinking` variants to `ContentBlock`

**File:** `src/anthropic/types.rs` (MODIFIED)

**Description:** Add new variants to the `ContentBlock` enum to handle `thinking` and `redacted_thinking` content blocks that appear in conversation history. These blocks are accepted during deserialization but stripped during translation to OpenAI format.

**Implementation:**

Add variants to `ContentBlock` enum (after `ToolResult`):
```rust
/// Thinking content block from a prior assistant turn.
///
/// Accepted during deserialization to avoid request failures when
/// conversation history includes thinking blocks. Stripped during
/// translation to OpenAI format (no equivalent exists).
#[serde(rename = "thinking")]
Thinking {
    thinking: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    signature: Option<String>,
},

/// Redacted thinking content block from a prior assistant turn.
///
/// Contains opaque base64-encoded data. Like `Thinking`, accepted
/// during deserialization and stripped during translation.
#[serde(rename = "redacted_thinking")]
RedactedThinking {
    data: String,
},
```

**Acceptance Criteria:**
- [ ] `{"type": "thinking", "thinking": "analysis text"}` deserializes to `ContentBlock::Thinking`
- [ ] `{"type": "thinking", "thinking": "text", "signature": "sig"}` deserializes (with optional signature)
- [ ] `{"type": "redacted_thinking", "data": "base64data"}` deserializes to `ContentBlock::RedactedThinking`
- [ ] Existing content block types (`text`, `image`, `document`, `tool_use`, `tool_result`) unaffected
- [ ] Full request with thinking blocks in conversation history deserializes without error

**Notes:** The `signature` field on `Thinking` is optional and may be present in some API versions. Including it prevents deserialization failures. The `RedactedThinking` variant uses `data` (opaque base64 content).

#### Task 4.3: Add `Reasoning` struct and field to `ChatCompletionRequest`

**File:** `src/copilot/types.rs` (MODIFIED)

**Description:** Add a `Reasoning` struct with an `effort` field, and add a `reasoning` field to `ChatCompletionRequest` for the OpenAI reasoning parameter.

**Implementation:**

Add `Reasoning` struct:
```rust
/// OpenAI reasoning configuration.
///
/// Controls model reasoning behavior. Currently only `effort` is supported.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Reasoning {
    /// Reasoning effort level.
    ///
    /// Model-dependent values: "none", "minimal", "low", "medium", "high", "xhigh".
    /// For Claude models via Copilot, typically "low", "medium", or "high".
    #[serde(skip_serializing_if = "Option::is_none")]
    pub effort: Option<String>,
}
```

Add field to `ChatCompletionRequest` (after `tool_choice`):
```rust
/// Reasoning configuration (effort level).
///
/// Translated from Anthropic's `output_config.effort`.
#[serde(skip_serializing_if = "Option::is_none")]
pub reasoning: Option<Reasoning>,
```

**Acceptance Criteria:**
- [ ] `ChatCompletionRequest` with `reasoning` serializes correctly
- [ ] `reasoning` field omitted from JSON when `None` (via `skip_serializing_if`)
- [ ] `Reasoning { effort: Some("high") }` serializes as `{"effort": "high"}`
- [ ] Existing `ChatCompletionRequest` construction sites compile (need `reasoning: None`)

#### Task 4.4: Update `to_chat_completion_request()` for effort translation and thinking handling

**File:** `src/anthropic/types.rs` (MODIFIED)

**Description:** Modify `to_chat_completion_request()` to:
1. Translate `output_config.effort` → `reasoning.effort` (with `"max"` → `"high"` mapping)
2. Strip `Thinking` and `RedactedThinking` content blocks from messages
3. Suppress temperature when `thinking` is present

**Implementation:**

**4.4a — Effort translation** (add before `ChatCompletionRequest` construction):
```rust
// Map Anthropic effort to OpenAI reasoning
let reasoning = self.output_config.as_ref()
    .and_then(|oc| oc.effort.as_ref())
    .map(|effort| {
        let mapped_effort = match effort.as_str() {
            "max" => "high".to_string(),
            other => other.to_string(),
        };
        tracing::debug!(
            anthropic_effort = %effort,
            openai_effort = %mapped_effort,
            "Translating effort level"
        );
        crate::copilot::types::Reasoning {
            effort: Some(mapped_effort),
        }
    });
```

**4.4b — Thinking block stripping** (add as a helper function):
```rust
/// Filter out thinking content blocks from a content block input.
///
/// Thinking and redacted_thinking blocks from prior assistant turns
/// have no OpenAI equivalent and must be stripped before translation.
fn strip_thinking_blocks(content: &ContentBlockInput) -> ContentBlockInput {
    match content {
        ContentBlockInput::Text(s) => ContentBlockInput::Text(s.clone()),
        ContentBlockInput::Blocks(blocks) => {
            let filtered: Vec<ContentBlock> = blocks.iter()
                .filter(|b| !matches!(
                    b,
                    ContentBlock::Thinking { .. } | ContentBlock::RedactedThinking { .. }
                ))
                .cloned()
                .collect();
            ContentBlockInput::Blocks(filtered)
        }
    }
}
```

Apply in the message translation loop:
```rust
for msg in &self.messages {
    let content = strip_thinking_blocks(&msg.content);
    // ... existing translation logic using `content` instead of `msg.content` ...
}
```

**4.4c — Temperature suppression:**
```rust
// Suppress temperature when thinking is active
let temperature = if self.thinking.is_some() {
    None
} else {
    self.temperature
};
```

**4.4d — Updated `ChatCompletionRequest` construction:**
```rust
ChatCompletionRequest {
    model: crate::model_mapper::normalize_model_name(&self.model),
    messages,
    stream: self.stream,
    temperature,          // may be None when thinking is active
    max_tokens: Some(self.max_tokens),
    top_p: self.top_p,
    n: None,
    stop,
    presence_penalty: None,
    frequency_penalty: None,
    tools: None,
    tool_choice: None,
    reasoning,            // NEW
}
```

**Acceptance Criteria:**
- [ ] `output_config.effort: "low"` → `reasoning.effort: "low"`
- [ ] `output_config.effort: "medium"` → `reasoning.effort: "medium"`
- [ ] `output_config.effort: "high"` → `reasoning.effort: "high"`
- [ ] `output_config.effort: "max"` → `reasoning.effort: "high"` (downgraded)
- [ ] No `output_config` → no `reasoning` field (backward compatible)
- [ ] `thinking` present → temperature is `None` in output
- [ ] `thinking` absent + temperature present → temperature forwarded
- [ ] Messages with `Thinking` content blocks → thinking blocks stripped, only text/tool blocks forwarded
- [ ] Messages with only thinking blocks (no text) → empty content, message skipped or produces empty text
- [ ] Existing message translation for text, image, tool_use, tool_result unaffected
- [ ] Debug log emitted for effort translation

**Notes:**
- The `strip_thinking_blocks` function must be called before all existing content inspection functions (`has_tool_result_blocks`, `has_multimodal_blocks`, `has_tool_use_blocks`, `extract_text`) to ensure thinking blocks don't interfere with translation logic.
- The temperature suppression is defensive — Claude Code already omits temperature when thinking is enabled, but the adapter should handle the case where both are present.
- Messages where all content blocks are stripped (only thinking blocks) should result in either an empty text message or be skipped entirely. Empty text messages are acceptable — the upstream API will handle them.

#### Task 4.5: Update all `ChatCompletionRequest` construction sites

**File:** Multiple files (MODIFIED)

**Description:** Anywhere `ChatCompletionRequest` is constructed directly (outside `to_chat_completion_request()`), add the new `reasoning: None` field. Search for `ChatCompletionRequest {` across the codebase to find all construction sites.

**Acceptance Criteria:**
- [ ] All `ChatCompletionRequest` construction sites include `reasoning` field
- [ ] Project compiles without errors
- [ ] No test regressions

---

### Epic 5: Testing (Day 2-3, ~0.75 day)

**Status:** Not Started

**Objective:** Ensure all four fixes are thoroughly tested with unit tests, integration tests, and documented manual E2E test procedures.

#### Task 5.1: Unit Tests — Error Translation

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

#### Task 5.2: Unit Tests — Copilot Error Parsing

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

#### Task 5.3: Unit Tests — 1M Context Beta Detection

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
   - [x] Test passes

2. **`has_1m_context_beta_absent`** — Header doesn't contain the beta:
   ```rust
   #[test]
   fn has_1m_context_beta_absent() {
       let mut headers = HeaderMap::new();
       headers.insert("anthropic-beta", "interleaved-thinking-2025-05-14".parse().unwrap());
       assert!(!has_1m_context_beta(&headers));
   }
   ```
   - [x] Test passes

3. **`has_1m_context_beta_no_header`** — No `anthropic-beta` header at all:
   ```rust
   #[test]
   fn has_1m_context_beta_no_header() {
       let headers = HeaderMap::new();
       assert!(!has_1m_context_beta(&headers));
   }
   ```
   - [x] Test passes

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
   - [x] Test passes

5. **`has_1m_context_beta_future_date`** — Forward-compatible with new date suffix:
   ```rust
   #[test]
   fn has_1m_context_beta_future_date() {
       let mut headers = HeaderMap::new();
       headers.insert("anthropic-beta", "context-1m-2026-12-31".parse().unwrap());
       assert!(has_1m_context_beta(&headers));
   }
   ```
   - [x] Test passes

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
   - [x] Test passes

7. **`model_name_no_double_append`** — Guard prevents double-appending:
   ```rust
   #[test]
   fn model_name_no_double_append() {
       let model = "claude-opus-4.6-1m";
       assert!(model.contains("-1m"));
       // Guard should prevent appending again
   }
   ```
   - [x] Test passes

**Acceptance Criteria:**
- [x] All 7 new 1M context tests pass
- [x] `has_1m_context_beta` is accessible to tests (either `pub(crate)` or tested via the handler)

**Notes:** If `has_1m_context_beta()` is private to the handler module, tests can either: (a) make it `pub(crate)` for test access, or (b) test it indirectly via integration tests (Task 4.6). Prefer (a) for faster feedback.

#### Task 5.4: Unit Tests — Streaming Truncation

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

#### Task 5.5: Unit Tests — Effort Translation and Thinking Blocks

**File:** `tests/unit/anthropic_types_tests.rs` (NEW or MODIFIED)

**Description:** Test effort translation, thinking block handling, and temperature suppression in `to_chat_completion_request()`.

**Tests to implement:**

1. **`effort_low_translates_to_reasoning_low`** — Direct mapping:
   ```rust
   #[test]
   fn effort_low_translates_to_reasoning_low() {
       let request = make_request_with_effort(Some("low"), None);
       let chat_req = request.to_chat_completion_request(false);
       assert_eq!(chat_req.reasoning.unwrap().effort.unwrap(), "low");
   }
   ```
   - [ ] Test passes

2. **`effort_max_translates_to_reasoning_high`** — Downgrade mapping:
   ```rust
   #[test]
   fn effort_max_translates_to_reasoning_high() {
       let request = make_request_with_effort(Some("max"), None);
       let chat_req = request.to_chat_completion_request(false);
       assert_eq!(chat_req.reasoning.unwrap().effort.unwrap(), "high");
   }
   ```
   - [ ] Test passes

3. **`no_effort_produces_no_reasoning`** — Backward compatibility:
   ```rust
   #[test]
   fn no_effort_produces_no_reasoning() {
       let request = make_request_with_effort(None, None);
       let chat_req = request.to_chat_completion_request(false);
       assert!(chat_req.reasoning.is_none());
   }
   ```
   - [ ] Test passes

4. **`thinking_present_suppresses_temperature`** — Temperature interaction:
   ```rust
   #[test]
   fn thinking_present_suppresses_temperature() {
       let request = make_request_with_thinking_and_temp(
           Some(json!({"type": "adaptive"})),
           Some(0.7),
       );
       let chat_req = request.to_chat_completion_request(false);
       assert!(chat_req.temperature.is_none());
   }
   ```
   - [ ] Test passes

5. **`thinking_absent_preserves_temperature`** — Normal temperature forwarding:
   ```rust
   #[test]
   fn thinking_absent_preserves_temperature() {
       let request = make_request_with_thinking_and_temp(None, Some(0.7));
       let chat_req = request.to_chat_completion_request(false);
       assert_eq!(chat_req.temperature, Some(0.7));
   }
   ```
   - [ ] Test passes

6. **`thinking_content_block_deserializes`** — serde acceptance:
   ```rust
   #[test]
   fn thinking_content_block_deserializes() {
       let json = json!({"type": "thinking", "thinking": "analysis"});
       let block: ContentBlock = serde_json::from_value(json).unwrap();
       assert!(matches!(block, ContentBlock::Thinking { .. }));
   }
   ```
   - [ ] Test passes

7. **`redacted_thinking_content_block_deserializes`**:
   ```rust
   #[test]
   fn redacted_thinking_content_block_deserializes() {
       let json = json!({"type": "redacted_thinking", "data": "base64data"});
       let block: ContentBlock = serde_json::from_value(json).unwrap();
       assert!(matches!(block, ContentBlock::RedactedThinking { .. }));
   }
   ```
   - [ ] Test passes

8. **`thinking_blocks_stripped_from_messages`** — Translation stripping:
   ```rust
   #[test]
   fn thinking_blocks_stripped_from_messages() {
       let request = make_request_with_thinking_blocks_in_history();
       let chat_req = request.to_chat_completion_request(false);
       // Verify assistant message content contains only the text, not thinking
       let assistant_msg = chat_req.messages.iter()
           .find(|m| m.role == "assistant").unwrap();
       match &assistant_msg.content {
           MessageContent::Text(t) => assert!(!t.contains("thinking")),
           _ => panic!("Expected text content"),
       }
   }
   ```
   - [ ] Test passes

9. **`request_with_output_config_deserializes`** — Full request deserialization:
   ```rust
   #[test]
   fn request_with_output_config_deserializes() {
       let json = json!({
           "model": "claude-opus-4-6",
           "max_tokens": 8192,
           "messages": [],
           "output_config": {"effort": "high"},
           "thinking": {"type": "adaptive"}
       });
       let request: AnthropicRequest = serde_json::from_value(json).unwrap();
       assert_eq!(request.output_config.unwrap().effort.unwrap(), "high");
       assert!(request.thinking.is_some());
   }
   ```
   - [ ] Test passes

**Acceptance Criteria:**
- [ ] All 9 effort/thinking tests pass
- [ ] Existing anthropic types tests still pass

#### Task 5.6: Integration Tests — Prompt-Too-Long Translation

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

#### Task 5.7: Integration Tests — Streaming Truncation

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

#### Task 5.8: Integration Tests — 1M Context Model Selection

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
- [x] 1M beta header → model name has `-1m` suffix
- [x] No beta header → model name unchanged
- [x] Existing integration message tests still pass

#### Task 5.9: Integration Tests — Effort Forwarding

**File:** `tests/integration/messages_tests.rs` (MODIFIED)

**Description:** Add an integration test that sends a request with `output_config.effort` through the adapter and verifies the mock Copilot API receives `reasoning.effort` in the request body.

**Scenario:**
1. **Setup:** Spawn a mock Copilot API that captures the request body.
2. **Action:** Send `POST /v1/messages` with `output_config: {"effort": "high"}` and `model: "claude-opus-4-6"`.
3. **Verification:**
   - The mock Copilot receives `reasoning: {"effort": "high"}` in the request body
   - The response is successful

- [ ] Test passes

**Acceptance Criteria:**
- [ ] Effort forwarding verified end-to-end
- [ ] Existing integration tests still pass

#### Task 5.10: Manual E2E Test Procedures

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

4. **Effort level forwarding:**
   ```
   1. Start copilot-adapter with --log-level trace
   2. Start Claude Code and run /effort high
   3. Send a message and observe the adapter trace logs
   4. Verify the adapter log shows "Translating effort level" with anthropic_effort="high" and openai_effort="high"
   5. Verify the outgoing request to Copilot API contains "reasoning":{"effort":"high"}
   6. Verify the conversation works normally
   7. Run /effort low and send another message
   8. Verify the outgoing request contains "reasoning":{"effort":"low"}
   ```
   - Expected: Effort level is forwarded to Copilot API in the `reasoning` object
   - [ ] Documented

5. **Thinking blocks in conversation history:**
   ```
   1. Start copilot-adapter with --log-level trace
   2. Start a Claude Code session (thinking should be enabled by default for supported models)
   3. Have a multi-turn conversation (at least 3 turns)
   4. Observe that subsequent requests include thinking blocks in conversation history
   5. Verify the adapter does NOT fail with deserialization errors
   6. Verify the outgoing requests to Copilot API do NOT contain thinking content blocks
   7. Verify the conversation continues normally
   ```
   - Expected: Thinking blocks accepted and stripped; no errors
   - [ ] Documented

**Acceptance Criteria:**
- [ ] All 5 E2E test procedures documented
- [ ] Steps are reproducible

---

### Epic 6: Documentation (Day 3, ~0.25 day)

**Status:** Not Started

**Objective:** Update project documentation to reflect the new error handling, streaming, effort, and thinking behavior.

#### Task 6.1: Update CLAUDE.md

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
- Add a note about effort and thinking support:
  - `output_config.effort` → `reasoning.effort` translation
  - Effort value mapping (`"max"` → `"high"`)
  - `thinking` and `redacted_thinking` content block handling (accepted, stripped)
  - Temperature suppression when thinking is active
  - `OutputConfig`, `Reasoning` structs

**Acceptance Criteria:**
- [ ] CLAUDE.md updated with prompt-too-long error handling info
- [ ] CLAUDE.md updated with truncation notice behavior
- [ ] CLAUDE.md updated with 1M context model activation info
- [ ] CLAUDE.md updated with effort/thinking support info

#### Task 6.2: Update known issues

**File:** `docs/known-issues.md` (MODIFIED)

**Changes:**
- If the prompt-too-long, truncation, and effort issues are listed as known issues, mark them as resolved with a reference to this implementation
- If not listed, add them as resolved items for historical reference

**Acceptance Criteria:**
- [ ] Known issues document updated

#### Task 6.3: Archive design document

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
| FR10 | `output_config.effort` → `reasoning.effort` in OpenAI request | Design doc §Option D | Epic 4 |
| FR11 | `"max"` effort maps to `"high"` in OpenAI format | Design doc §Option D.6a | Epic 4 |
| FR12 | No `output_config` → no `reasoning` field (backward compatible) | Design doc §Option D.8 | Epic 4 |
| FR13 | `thinking` and `redacted_thinking` content blocks accepted in conversation history | Design doc §Option D.3 | Epic 4 |
| FR14 | Thinking content blocks stripped during translation to OpenAI format | Design doc §Option D.6b | Epic 4 |
| FR15 | Temperature suppressed when `thinking` parameter is present | Design doc §Option D.6c | Epic 4 |

### Non-Functional Requirements

| ID | Requirement | Target | Epic |
|----|-------------|--------|------|
| NFR1 | No new crate dependencies | Zero new dependencies | All |
| NFR2 | Error translation adds negligible latency | <1ms additional parsing | Epic 1 |
| NFR3 | No changes to non-error streaming paths | Identical SSE output for successful requests | Epic 3 |
| NFR4 | Beta header detection is forward-compatible | Prefix match `context-1m` works for future dates | Epic 2 |
| NFR5 | Effort translation adds negligible latency | <1ms additional mapping | Epic 4 |
| NFR6 | Backward-compatible with requests lacking effort/thinking fields | Existing requests produce identical output | Epic 4 |

---

## File Changes Summary

| File | Change | Epic | Description |
|------|--------|------|-------------|
| `src/error.rs` | Modified | Epic 1 | Add `PromptTooLong` variant, HTTP 400 mapping, `error_type()` arm |
| `src/copilot/client.rs` | Modified | Epic 1 | Add `parse_prompt_too_long()` function, update `handle_error_response()` |
| `src/handlers/messages.rs` | Modified | Epic 2 | Add `HeaderMap` extraction, `has_1m_context_beta()`, append `-1m` to model name |
| `src/streaming/state.rs` | Modified | Epic 3 | Add `has_emitted_tool_use` field, emit text block on truncation |
| `src/anthropic/types.rs` | Modified | Epic 4 | Add `OutputConfig` struct, `output_config`/`thinking` fields, `Thinking`/`RedactedThinking` content block variants, `strip_thinking_blocks()`, effort→reasoning translation, temperature suppression |
| `src/copilot/types.rs` | Modified | Epic 4 | Add `Reasoning` struct, `reasoning` field on `ChatCompletionRequest` |
| `tests/unit/error_tests.rs` | Modified | Epic 5 | Add 3 tests: HTTP response format, regex match, error_type |
| `tests/unit/copilot_client_tests.rs` | Modified | Epic 5 | Add 6 tests: prompt-too-long parsing (valid, edge cases, failures) |
| `tests/unit/messages_tests.rs` | New/Modified | Epic 5 | Add 7 tests: 1M beta detection, model name transformation |
| `tests/unit/streaming_tests.rs` | Modified | Epic 5 | Update 4 existing truncation tests, add 2 new edge-case tests |
| `tests/unit/anthropic_types_tests.rs` | New/Modified | Epic 5 | Add 9 tests: effort translation, thinking block deserialization/stripping, temperature suppression |
| `tests/integration/error_tests.rs` | Modified | Epic 5 | Add 1 integration test: prompt-too-long end-to-end |
| `tests/integration/messages_tests.rs` | Modified | Epic 5 | Add 2 integration tests: 1M model selection + effort forwarding |
| `tests/integration/streaming_tests.rs` | Modified/New | Epic 5 | Add 1 integration test: truncation notice in SSE stream |
| `docs/development/e2e-testing.md` | Modified | Epic 5 | Add 5 manual E2E test procedures |
| `CLAUDE.md` | Modified | Epic 6 | Add development notes for prompt-too-long, 1M context, effort/thinking, and truncation handling |
| `docs/known-issues.md` | Modified | Epic 6 | Update/add resolved issue entries |
| `docs/design/CONTEXT-WINDOW-AND-TRUNCATION.design.md` | Modified | Epic 6 | Update status to "Implemented" |

---

## Testing Strategy

### Test Coverage

| Component | Unit Tests | Integration Tests | E2E Tests |
|-----------|------------|-------------------|-----------|
| `PromptTooLong` error variant | Task 5.1 (3 tests) | Task 5.6 (1 test) | Task 5.10 (procedure 1) |
| `parse_prompt_too_long()` parser | Task 5.2 (6 tests) | Task 5.6 (1 test) | — |
| `handle_error_response()` 400 path | — | Task 5.6 (1 test) | Task 5.10 (procedure 1) |
| `has_1m_context_beta()` detection | Task 5.3 (7 tests) | Task 5.8 (2 tests) | Task 5.10 (procedure 3) |
| Model name `-1m` appending | Task 5.3 (2 tests) | Task 5.8 (2 tests) | Task 5.10 (procedure 3) |
| Truncation text notice | Task 5.4 (6 tests) | Task 5.7 (1 test) | Task 5.10 (procedure 2) |
| `has_emitted_tool_use` tracking | Task 5.4 (implicit) | — | — |
| Effort translation | Task 5.5 (4 tests) | Task 5.9 (1 test) | Task 5.10 (procedure 4) |
| Thinking block deserialization | Task 5.5 (3 tests) | — | Task 5.10 (procedure 5) |
| Thinking block stripping | Task 5.5 (1 test) | — | Task 5.10 (procedure 5) |
| Temperature suppression | Task 5.5 (2 tests) | — | — |
| `OutputConfig` deserialization | Task 5.5 (1 test) | — | — |

### Test Files

| File | Type | Coverage |
|------|------|----------|
| `tests/unit/error_tests.rs` | Unit | PromptTooLong error format + regex match |
| `tests/unit/copilot_client_tests.rs` | Unit | Error body parsing (6 cases) |
| `tests/unit/messages_tests.rs` | Unit | 1M beta detection + model name transformation (7 tests) |
| `tests/unit/streaming_tests.rs` | Unit | Truncation notice emission (6 tests) |
| `tests/unit/anthropic_types_tests.rs` | Unit | Effort translation, thinking blocks, temperature (9 tests) |
| `tests/integration/error_tests.rs` | Integration | Full HTTP round-trip for prompt-too-long |
| `tests/integration/messages_tests.rs` | Integration | 1M model selection (2 scenarios) + effort forwarding (1 scenario) |
| `tests/integration/streaming_tests.rs` | Integration | Full SSE stream with truncation |
| `docs/development/e2e-testing.md` | Manual E2E | Long conversation + large file write + 1M context + effort forwarding + thinking blocks |

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
| `src/anthropic/types.rs` (`AnthropicRequest`) | Epic 4 | ✅ Exists — adding `output_config`, `thinking` fields |
| `src/anthropic/types.rs` (`ContentBlock`) | Epic 4 | ✅ Exists — adding `Thinking`, `RedactedThinking` variants |
| `src/copilot/types.rs` (`ChatCompletionRequest`) | Epic 4 | ✅ Exists — adding `reasoning` field |
| `serde_json::Value` | Epic 4 | ✅ Exists — already used throughout codebase |

---

## Risk Assessment

| Risk | Impact | Probability | Mitigation | Epic |
|------|--------|-------------|------------|------|
| Anthropic SDK doesn't match `prompt is too long` in JSON-stringified body | High | Low | Task 5.1 test 2 validates exact SDK behavior with regex | Epic 1 |
| Copilot API changes error message format | Medium | Low | `parse_prompt_too_long` returns `None` → falls through to generic `CopilotError` | Epic 1 |
| Copilot API doesn't have 1M variant for a model Claude Code requests | Medium | Medium | API returns a clear error for unknown model IDs; no silent downgrade | Epic 2 |
| Beta header date format changes | Low | Low | Prefix matching on `context-1m` is forward-compatible | Epic 2 |
| Non-Claude model gets `-1m` appended | Low | Very Low | Claude Code only sends `context-1m-*` for Claude models; Copilot rejects unknown IDs | Epic 2 |
| Truncation text block confuses the model on retry | Low | Medium | Square-bracket format is clearly system-level; model has seen similar patterns | Epic 3 |
| Updated streaming tests break other streaming tests | Medium | Low | Additive changes only; existing event sequences extended, not replaced | Epic 5 |
| `has_emitted_tool_use` tracking has off-by-one or state leak | Low | Low | Simple boolean, set in one place, tested explicitly | Epic 3 |
| Copilot API 400 errors from other causes wrongly matched as prompt-too-long | Low | Very Low | Checks `code` field specifically, not message text | Epic 1 |
| Copilot API ignores `reasoning.effort` for Claude models | Medium | Medium | If ignored, behavior is unchanged from today (default effort). Verifiable via trace logging. | Epic 4 |
| Copilot API rejects `reasoning.effort` for Claude models | Medium | Low | `skip_serializing_if = "Option::is_none"` means absent effort produces no field; `--disable-reasoning` flag can be added as escape hatch. | Epic 4 |
| Unknown content block types beyond `thinking`/`redacted_thinking` | Low | Low | serde still fails on truly unknown types; can add more variants as discovered | Epic 4 |
| Thinking blocks needed for model context | Low | Medium | Thinking blocks are internal reasoning artifacts; stripping is consistent with 3P proxy behavior | Epic 4 |

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
- [x] ~~Add `has_emitted_tool_use` field (Task 3.1)~~ (Removed — dead-write field)
- [x] Implement truncation text block emission (Task 3.2)
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
| Epic 2: 1M Context Model Activation | Complete | - | - | 3 tasks |
| Epic 3: Truncated Tool Call Recovery | Complete | - | - | 2 tasks |
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
- `has_1m_context_beta()` must use prefix matching (`context-1m`) not exact string matching, to be forward-compatible with future beta date suffixes
- The `-1m` suffix must be appended AFTER `normalize_model_name()`, not before, to ensure correct Copilot model ID format
- The guard `!model.contains("-1m")` must prevent double-appending in all code paths

### Development Notes
- [Notes added during implementation]

### Review Notes
- [Code review feedback]

### Testing Notes
- [Test failures and fixes]
- [Edge cases discovered]
