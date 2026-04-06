# Usage Token Counts in Streaming Responses — Implementation Plan

**Status:** In Progress
**Date:** 2026-04-06
**Based on:** [USAGE-TOKEN-COUNTS.design.md](./USAGE-TOKEN-COUNTS.design.md)
**Prerequisite:** None
**Estimated Time:** 1–2 days

---

## Executive Summary

Claude Code's `/model` view displays a token usage header (`N/1m tokens (P%)`) derived from the `usage` field in the Anthropic `message_start` and `message_delta` SSE events. The adapter currently hardcodes `input_tokens: 0` and `output_tokens: 0` in every streaming response because the GitHub Copilot API never returns usage data in its SSE chunks. This means the header always shows `0/Nm tokens (0%)`.

This plan implements local token counting using the `tiktoken-rs` crate already present in the codebase: count input tokens via `count_tokens_for_request()` before the stream begins, and count output tokens by accumulating streamed text/tool-call fragments in `StreamingState` and running `count_output_tokens()` at finalization. As a forward-compatibility bonus, `ChatCompletionChunk` gains an optional `usage` field so real counts from the API take precedence if the Copilot API ever starts returning them.

This plan implements:
- `count_tokens_for_request()` and `count_output_tokens()` helpers in `src/token_counter.rs`
- `ChatCompletionChunk.usage` optional field for upstream usage passthrough
- `build_message_start_response()` updated to accept an `input_tokens` parameter
- `StreamingState` gains `input_tokens`, `output_text`, `output_tool_json`, and optional upstream usage fields; emits real counts in `message_start` and `message_delta`
- Both streaming paths in `src/handlers/messages.rs` compute `input_tokens` before entering the stream
- Unit and integration test coverage for all new behaviour

**Total estimated time:** 1–2 days

---

## Background

### Current State

- `build_message_start_response()` in `src/anthropic/types.rs:942` hardcodes `usage: AnthropicUsage { input_tokens: 0, output_tokens: 0 }`
- `handle_finish()` in `src/streaming/state.rs:425` hardcodes `usage: MessageDeltaUsage { output_tokens: 0 }` with a TODO comment
- `finalize()` in `src/streaming/state.rs:184` also hardcodes `output_tokens: 0`
- `ChatCompletionChunk` in `src/copilot/types.rs` has no `usage` field
- `src/token_counter.rs` already contains `count_tokens()` that works on `CountTokensRequest`; it uses `cl100k_base` BPE encoding, the same encoder needed here
- `StreamingState::new()` takes only `name_mapping: HashMap<String, String>`

### Target State

- `message_start.message.usage.input_tokens` carries the tiktoken estimate of the full request (system + messages + tools)
- `message_delta.usage.output_tokens` carries the tiktoken estimate of the accumulated response (text + tool call JSON)
- If the Copilot API returns a `usage` field in a streaming chunk, those real counts take precedence over the tiktoken estimates
- Claude Code's `/model` view header shows non-zero counts after every response
- All behaviour degrades gracefully to `0` if the tiktoken encoder fails to initialize

---

## Goals and Non-Goals

### Goals

| ID | Goal | Success Criteria |
|----|------|------------------|
| G1 | Emit real `input_tokens` in `message_start` | Header shows non-zero count matching `count_tokens` estimate within ~5% |
| G2 | Emit real `output_tokens` in `message_delta` | Header total increments correctly after each turn |
| G3 | No new crate dependencies | `Cargo.toml` unchanged (tiktoken-rs already present) |
| G4 | No increase in time-to-first-token | Input counting completes before stream begins; output counting is incremental |
| G5 | Works for both native tools and XML injection paths | Both code paths pass token counts through correctly |
| G6 | Forward-compatibility with upstream usage | `ChatCompletionChunk.usage` is parsed when present and takes precedence |

### Non-Goals

| ID | Non-Goal | Rationale |
|----|----------|-----------|
| NG1 | Exact token counts matching Copilot API's own counts | Copilot API doesn't expose them; approximations are sufficient for UX |
| NG2 | Caching the tiktoken encoder in `AppState` | Existing code doesn't; defer until profiling shows need |
| NG3 | Per-model tokenizer selection | All Copilot Claude models use `cl100k_base`; accurate enough |
| NG4 | Counting cache creation / cache read tokens | Copilot doesn't support prompt caching; these fields stay 0 |
| NG5 | Non-streaming `input_tokens` fallback | Non-streaming Copilot responses include `usage` in the body already |

---

## Implementation Plan

### Epic 1: Core token counting helpers (Day 1, ~2 hours)

**Status:** DONE

**Objective:** Add two new public functions to `src/token_counter.rs` that the streaming layer can call.

#### Task 1.1: Add `count_tokens_for_request()` and `count_output_tokens()` to `token_counter.rs`

**File:** `src/token_counter.rs` (MODIFIED)

**Description:** Add two public functions. `count_tokens_for_request()` is a thin wrapper around the existing `count_tokens()` — it constructs a `CountTokensRequest` from an `&AnthropicRequest` and delegates. `count_output_tokens()` is a standalone BPE-encode of an arbitrary `&str`. Both return `0` on encoder failure rather than propagating errors.

**Implementation:**
```rust
/// Count input tokens for an [`AnthropicRequest`].
///
/// Reuses the same `cl100k_base` BPE encoder as the `count_tokens` endpoint.
/// Returns 0 on encoder failure — token counting is best-effort and must not
/// block the request.
pub fn count_tokens_for_request(request: &AnthropicRequest) -> u32 {
    let count_request = CountTokensRequest {
        model: request.model.clone(),
        messages: request.messages.clone(),
        system: request.system.clone(),
        tools: request.tools.clone(),
    };
    count_tokens(&count_request).unwrap_or(0)
}

/// Count output tokens for a completed response string.
///
/// Tokenizes `text` using `cl100k_base`. Returns 0 on encoder failure.
pub fn count_output_tokens(text: &str) -> u32 {
    match cl100k_base() {
        Ok(bpe) => bpe.encode_with_special_tokens(text).len() as u32,
        Err(_) => 0,
    }
}
```

**Import needed in `token_counter.rs`:**
```rust
use crate::anthropic::types::AnthropicRequest;
```

**Acceptance Criteria:**
- [x] `count_tokens_for_request()` returns the same value as `count_tokens()` for an equivalent `CountTokensRequest`
- [x] `count_output_tokens("")` returns 0
- [x] `count_output_tokens("Hello, world!")` returns > 0
- [x] Both functions return 0 (not panic / error) if called when the BPE encoder fails

---

### Epic 2: Add optional `usage` field to `ChatCompletionChunk` (Day 1, ~30 min)

**Status:** Not Started

**Objective:** Future-proof the streaming struct so real usage from upstream takes precedence over tiktoken estimates when the Copilot API eventually starts returning it.

#### Task 2.1: Add `usage: Option<Usage>` to `ChatCompletionChunk`

**File:** `src/copilot/types.rs` (MODIFIED)

**Description:** Append an optional `usage` field to `ChatCompletionChunk`. The existing `Usage` struct (already used by `ChatCompletion`) is reused.

**Implementation:**
```rust
/// A streaming chat completion chunk (OpenAI-compatible SSE format).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatCompletionChunk {
    pub id: String,
    #[serde(default = "default_chunk_object_type")]
    pub object: String,
    pub created: i64,
    pub model: String,
    pub choices: Vec<ChunkChoice>,
    /// Token usage statistics. Most providers (including GitHub Copilot today)
    /// omit this field from streaming chunks; when present it takes precedence
    /// over the local tiktoken estimate.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub usage: Option<Usage>,
}
```

**Acceptance Criteria:**
- [ ] `ChatCompletionChunk` deserializes correctly when `usage` is absent (existing behaviour unchanged)
- [ ] `ChatCompletionChunk` deserializes correctly when `usage` is present (e.g. `{"usage": {"prompt_tokens": 10, "completion_tokens": 5, "total_tokens": 15}}`)
- [ ] Existing unit tests in `tests/unit/copilot_types_tests.rs` still pass

---

### Epic 3: Update `build_message_start_response()` (Day 1, ~30 min)

**Status:** Not Started

**Objective:** Allow the function to emit a real `input_tokens` count instead of always 0.

#### Task 3.1: Add `input_tokens` parameter to `build_message_start_response()`

**File:** `src/anthropic/types.rs` (MODIFIED)

**Description:** Add an `input_tokens: u32` parameter. Update the `usage` field initialisation to use it. Update the only call site (`build_message_start()` in `src/streaming/state.rs`) to pass the stored count.

**Implementation:**
```rust
/// Build the initial [`AnthropicResponse`] shell used in the `message_start`
/// streaming event.
///
/// `input_tokens` is the tiktoken estimate for the full request;
/// `output_tokens` starts at zero and is updated by the `message_delta` event.
pub fn build_message_start_response(id: &str, model: &str, input_tokens: u32) -> AnthropicResponse {
    AnthropicResponse {
        id: format!("msg_{}", id.trim_start_matches("chatcmpl-")),
        response_type: "message".to_string(),
        role: "assistant".to_string(),
        content: vec![],
        model: model.to_string(),
        stop_reason: None,
        stop_sequence: None,
        usage: AnthropicUsage {
            input_tokens,
            output_tokens: 0,
        },
    }
}
```

**Acceptance Criteria:**
- [ ] Function signature updated to `(id: &str, model: &str, input_tokens: u32) -> AnthropicResponse`
- [ ] Returned `usage.input_tokens` equals the passed-in value
- [ ] Returned `usage.output_tokens` is always 0
- [ ] Compiler confirms no other call sites remain with the old two-argument form
- [ ] Existing unit tests in `tests/unit/anthropic_types_tests.rs` updated and passing

---

### Epic 4: Update `StreamingState` to track and emit real counts (Day 1, ~3 hours)

**Status:** Not Started

**Objective:** `StreamingState` gains fields for `input_tokens`, accumulated output text/tool JSON, and optional upstream usage override. All emission points are updated to use real values.

#### Task 4.1: Add new fields to `StreamingState` struct

**File:** `src/streaming/state.rs` (MODIFIED)

**Description:** Add five new fields to the struct. The first two store token counts; the next two accumulate output content; the last two capture upstream usage if the API returns it.

```rust
pub struct StreamingState {
    // ... existing fields unchanged ...

    /// Pre-computed input token count from the Anthropic request.
    /// Injected into the `message_start` event.
    input_tokens: u32,

    /// Accumulated output text for token counting at stream end.
    output_text: String,

    /// Accumulated tool call argument JSON for token counting at stream end.
    output_tool_json: String,

    /// Real input token count from the upstream API (if provided).
    upstream_input_tokens: Option<u32>,

    /// Real output token count from the upstream API (if provided).
    upstream_output_tokens: Option<u32>,
}
```

**Acceptance Criteria:**
- [ ] All five fields present in the struct definition
- [ ] `StreamingState::new()` signature updated to `(name_mapping: HashMap<String, String>, input_tokens: u32) -> Self`
- [ ] `new()` initialises `output_text` and `output_tool_json` as empty `String::new()`, upstream fields as `None`

#### Task 4.2: Accumulate output text in `handle_text_delta()`

**File:** `src/streaming/state.rs` (MODIFIED)

**Description:** Append each text fragment to `self.output_text` as it arrives. The accumulation must happen *after* the emptiness check to be consistent.

```rust
fn handle_text_delta(&mut self, text: &str) -> Vec<StreamEvent> {
    // ... existing block-switching logic unchanged ...

    // Accumulate for output token counting.
    self.output_text.push_str(text);

    // Emit delta event.
    // ... existing push of ContentBlockDelta unchanged ...
}
```

**Acceptance Criteria:**
- [ ] After processing N text chunks, `self.output_text` equals the concatenation of all non-empty text values
- [ ] Empty-string text chunks (already filtered by the caller) do not contribute

#### Task 4.3: Accumulate tool call argument JSON in `handle_tool_call_delta()`

**File:** `src/streaming/state.rs` (MODIFIED)

**Description:** Append non-empty argument fragments from each tool call delta to `self.output_tool_json`.

```rust
// Inside the loop over tool call deltas, after existing name/id extraction:
if let Some(func) = &tc.function {
    if let Some(args) = &func.arguments {
        if !args.is_empty() {
            self.output_tool_json.push_str(args);
        }
    }
}
```

**Acceptance Criteria:**
- [ ] After processing a tool call with three argument fragments `{"a":`, `1`, `}`, `self.output_tool_json` equals `{"a":1}`

#### Task 4.4: Capture upstream usage in `process_chunk()`

**File:** `src/streaming/state.rs` (MODIFIED)

**Description:** After processing a chunk's choices, check `chunk.usage` and store it if present.

```rust
// At the end of process_chunk(), after the choices loop:
if let Some(upstream_usage) = &chunk.usage {
    self.upstream_output_tokens = Some(upstream_usage.completion_tokens);
    self.upstream_input_tokens = Some(upstream_usage.prompt_tokens);
}
```

**Acceptance Criteria:**
- [ ] When a chunk carries `usage: { prompt_tokens: 100, completion_tokens: 50 }`, `self.upstream_input_tokens == Some(100)` and `self.upstream_output_tokens == Some(50)` after processing that chunk

#### Task 4.5: Add `compute_output_tokens()` and `compute_input_tokens()` helpers

**File:** `src/streaming/state.rs` (MODIFIED)

**Description:** Two private helpers that prefer upstream usage when available, falling back to tiktoken estimates.

```rust
fn compute_output_tokens(&self) -> u32 {
    if let Some(tokens) = self.upstream_output_tokens {
        return tokens;
    }
    let combined = format!("{}{}", self.output_text, self.output_tool_json);
    token_counter::count_output_tokens(&combined)
}

fn compute_input_tokens(&self) -> u32 {
    self.upstream_input_tokens.unwrap_or(self.input_tokens)
}
```

**Acceptance Criteria:**
- [ ] `compute_output_tokens()` returns `upstream_output_tokens` when set
- [ ] `compute_output_tokens()` returns tiktoken estimate of `output_text + output_tool_json` when upstream absent
- [ ] `compute_input_tokens()` returns `upstream_input_tokens` when set
- [ ] `compute_input_tokens()` returns `self.input_tokens` when upstream absent

#### Task 4.6: Update `build_message_start()` to pass real `input_tokens`

**File:** `src/streaming/state.rs` (MODIFIED)

**Description:** Pass `self.compute_input_tokens()` to the updated `build_message_start_response()`.

```rust
fn build_message_start(&self) -> StreamEvent {
    StreamEvent::MessageStart {
        message: build_message_start_response(
            self.message_id.as_deref().unwrap_or("unknown"),
            self.model.as_deref().unwrap_or("unknown"),
            self.compute_input_tokens(),
        ),
    }
}
```

**Acceptance Criteria:**
- [ ] `message_start.message.usage.input_tokens` equals the value passed into `StreamingState::new()` when no upstream usage is present
- [ ] `message_start.message.usage.input_tokens` equals the upstream `prompt_tokens` when upstream usage is present

#### Task 4.7: Update `handle_finish()` and `finalize()` to emit real `output_tokens`

**File:** `src/streaming/state.rs` (MODIFIED)

**Description:** Replace the two `output_tokens: 0` hardcodes with `self.compute_output_tokens()`.

In `handle_finish()`:
```rust
events.push(StreamEvent::MessageDelta {
    delta: MessageDeltaBody { stop_reason, stop_sequence: None },
    usage: MessageDeltaUsage {
        output_tokens: self.compute_output_tokens(),
    },
});
```

In `finalize()`:
```rust
events.push(StreamEvent::MessageDelta {
    delta: MessageDeltaBody {
        stop_reason: Some("end_turn".to_string()),
        stop_sequence: None,
    },
    usage: MessageDeltaUsage { output_tokens: self.compute_output_tokens() },
});
```

Also remove the TODO comment from `handle_finish()`.

**Acceptance Criteria:**
- [ ] `message_delta.usage.output_tokens > 0` after any streaming response that contained text
- [ ] `message_delta.usage.output_tokens > 0` after a tool call response
- [ ] After a `finish_reason: "length"` truncation where the tool call buffer is discarded, `output_tokens` reflects only the truncation notice text (not the discarded arguments)

---

### Epic 5: Wire input counting in `src/handlers/messages.rs` (Day 1–2, ~2 hours)

**Status:** Not Started

**Objective:** Both streaming paths in the messages handler compute `input_tokens` before constructing `StreamingState`.

#### Task 5.1: Add import and compute input tokens in `handle_native_tools_streaming()`

**File:** `src/handlers/messages.rs` (MODIFIED)

**Description:** `handle_native_tools_streaming()` currently calls `StreamingState::new(name_mapping.clone())`. The updated signature requires `input_tokens`. The handler has access to the original `AnthropicRequest` (passed through the call chain) so `count_tokens_for_request()` can be called there.

Identify where `handle_native_tools_streaming()` is called and what `AnthropicRequest` value is in scope. Add a `input_tokens: u32` parameter to the function or compute it locally from the request before the `async_stream::stream!` block.

**Implementation sketch:**
```rust
// At the top of the streaming event_stream block (or just before it):
let input_tokens = token_counter::count_tokens_for_request(&anthropic_request);

// ...

let mut streaming_state = StreamingState::new(name_mapping.clone(), input_tokens);
```

**Acceptance Criteria:**
- [ ] `input_tokens` is computed exactly once, before the `async_stream::stream!` block begins
- [ ] Value is passed into `StreamingState::new()`
- [ ] No performance regression: computation must complete before the first chunk is requested from upstream

#### Task 5.2: Wire input counting for the XML injection streaming path (`handle_streaming()` / `handle_streaming_with_tools()`)

**File:** `src/handlers/messages.rs` (MODIFIED)

**Description:** `handle_streaming()` and `handle_streaming_with_tools()` are the XML injection paths. These also use manual streaming loops (without `StreamingState`). Examine whether these paths use `StreamingState` or their own inline state. If they do not use `StreamingState`, they need their own `message_start` fix.

From reading the code, `handle_streaming_with_tools()` buffers all chunks and emits events manually — it does NOT use `StreamingState`. `handle_streaming()` (the non-tool path) also appears to use its own inline loop. These paths must be updated separately to pass real `input_tokens` into their own `build_message_start_response()` calls.

For each path:
1. Determine the location of `build_message_start_response()` calls
2. Add `let input_tokens = token_counter::count_tokens_for_request(&request);` before the stream
3. Pass `input_tokens` to `build_message_start_response()`
4. Accumulate output text and call `count_output_tokens()` at finalization

**Acceptance Criteria:**
- [ ] All call sites of `build_message_start_response()` pass a non-zero `input_tokens` for a real request
- [ ] `message_delta.usage.output_tokens` is non-zero in the XML injection streaming path after a response
- [ ] Compiler confirms all callers of `build_message_start_response()` have been updated (three-argument form required)

**Notes:** This task may be more involved than it looks. Read `handle_streaming()` and `handle_streaming_with_tools()` fully before implementing to identify all the SSE emission points.

---

### Epic 6: Testing (Day 2, ~3 hours)

**Status:** Not Started

**Objective:** Comprehensive unit and integration test coverage for all new token counting behaviour.

#### Task 6.1: Unit tests for new `token_counter.rs` helpers

**File:** `tests/unit/token_counter_tests.rs` (MODIFIED)

**Tests to add:**
1. **`count_tokens_for_request_matches_count_tokens()`** — Construct identical `AnthropicRequest` and `CountTokensRequest`; verify both functions return the same value.
2. **`count_output_tokens_empty_string()`** — `count_output_tokens("")` returns 0.
3. **`count_output_tokens_nonempty_string()`** — `count_output_tokens("Hello, world!")` returns > 0.
4. **`count_output_tokens_increases_with_length()`** — `count_output_tokens(long_text) > count_output_tokens(short_text)`.

**Acceptance Criteria:**
- [ ] All four tests pass
- [ ] No existing tests broken

#### Task 6.2: Unit tests for `StreamingState` token counting

**File:** `tests/unit/streaming_tests.rs` (MODIFIED)

**Tests to add:**
1. **`message_start_carries_input_tokens()`** — `StreamingState::new(HashMap::new(), 42)` produces a `message_start` with `usage.input_tokens: 42` on the first chunk.
2. **`message_start_zero_input_tokens_ok()`** — `StreamingState::new(HashMap::new(), 0)` produces `usage.input_tokens: 0` (no regression).
3. **`message_delta_carries_output_tokens_after_text()`** — After processing multiple text chunks and calling `finalize()`, the emitted `message_delta.usage.output_tokens > 0`.
4. **`message_delta_carries_output_tokens_after_tool_call()`** — After processing a tool call chunk (with argument fragments), `handle_finish()` emits `output_tokens > 0`.
5. **`output_tokens_increase_with_response_length()`** — A longer text response produces a higher `output_tokens` than a shorter one.
6. **`upstream_usage_overrides_tiktoken_for_output()`** — When a chunk carries `usage: { prompt_tokens: 100, completion_tokens: 50 }`, `message_delta` emits `output_tokens: 50`.
7. **`upstream_usage_overrides_tiktoken_for_input()`** — Same chunk: `message_start` emits `input_tokens: 100`.
8. **`truncated_tool_call_output_tokens_from_notice_text()`** — After a `finish_reason: "length"` truncation that discards a tool call buffer and emits a notice, `output_tokens` reflects only the notice text length.

**Acceptance Criteria:**
- [ ] All eight tests pass
- [ ] All pre-existing tests in `streaming_tests.rs` still pass

#### Task 6.3: Unit tests for `build_message_start_response()` signature change

**File:** `tests/unit/anthropic_types_tests.rs` (MODIFIED)

**Tests to add / update:**
1. Update any existing calls to `build_message_start_response()` to pass `0` as the third argument.
2. **`build_message_start_response_passes_input_tokens()`** — Called with `input_tokens: 1234`, returned `usage.input_tokens == 1234`.
3. **`build_message_start_response_output_tokens_always_zero()`** — Returned `usage.output_tokens == 0` regardless of `input_tokens`.

**Acceptance Criteria:**
- [ ] All existing tests updated and passing
- [ ] Two new tests pass

#### Task 6.4: Integration test for streaming usage fields

**File:** `tests/integration/streaming_tests.rs` (MODIFIED)

**Scenarios to add:**
1. **`streaming_response_has_nonzero_input_tokens()`**
   - Setup: Mock Copilot API returning a simple text streaming response (no `usage` in chunks)
   - Action: Send a streaming chat request; collect all SSE events
   - Verification: `message_start.message.usage.input_tokens > 0`
   - [ ] Test passes

2. **`streaming_response_has_nonzero_output_tokens()`**
   - Setup: Same mock
   - Action: Collect all SSE events
   - Verification: `message_delta.usage.output_tokens > 0`
   - [ ] Test passes

3. **`streaming_input_token_count_consistent_with_count_tokens_endpoint()`**
   - Setup: Send identical body to both `POST /v1/messages/count_tokens` and `POST /v1/messages` (streaming)
   - Action: Compare responses
   - Verification: `count_tokens` response `input_tokens` equals `message_start.usage.input_tokens`
   - [ ] Test passes

4. **`upstream_usage_in_chunk_overrides_tiktoken()`**
   - Setup: Mock Copilot API that returns `usage: { prompt_tokens: 999, completion_tokens: 888 }` in the final streaming chunk
   - Verification: `message_start.usage.input_tokens == 999` and `message_delta.usage.output_tokens == 888`
   - [ ] Test passes

**Acceptance Criteria:**
- [ ] All four new integration scenarios pass
- [ ] All existing integration tests in `streaming_tests.rs` still pass

---

### Epic 7: Documentation (Day 2, ~30 min)

**Status:** Not Started

**Objective:** Update `CLAUDE.md` and `docs/known-issues.md` to reflect the new behaviour.

#### Task 7.1: Update `CLAUDE.md` — Token counting note

**File:** `CLAUDE.md` (MODIFIED)

**Changes:**
- Update the "Token counting" bullet to note that streaming responses now carry real `input_tokens` and `output_tokens` estimated via `cl100k_base` tiktoken
- Clarify that `count_tokens_for_request()` and `count_output_tokens()` are the new helpers in `src/token_counter.rs`

**Acceptance Criteria:**
- [ ] No longer implies streaming usage is always zero

#### Task 7.2: Update `CLAUDE.md` — Streaming state machine note

**File:** `CLAUDE.md` (MODIFIED)

**Changes:**
- Add a sentence to the "Streaming state machine" note describing `output_text` / `output_tool_json` accumulation and tiktoken counting at finalization
- Mention the upstream usage override path (`ChatCompletionChunk.usage`)

**Acceptance Criteria:**
- [ ] Note updated

#### Task 7.3: Update `docs/known-issues.md`

**File:** `docs/known-issues.md` (MODIFIED)

**Changes:**
- Remove or resolve any entry that states the `/model` view always shows zero token counts

**Acceptance Criteria:**
- [ ] Known-issues file no longer lists zero token counts as an open issue

---

## Requirements

### Functional Requirements

| ID | Requirement | Source | Epic |
|----|-------------|--------|------|
| FR1 | `message_start.message.usage.input_tokens` must be non-zero for any non-empty request | Design §Problem Statement | Epic 4, 5 |
| FR2 | `message_delta.usage.output_tokens` must be non-zero after a text response | Design §Problem Statement | Epic 4, 5 |
| FR3 | Counts must be consistent with those returned by `POST /v1/messages/count_tokens` | Design §FR3 | Epic 1, 6 |
| FR4 | If the Copilot API returns usage in a chunk, it must take precedence over tiktoken | Design §FR4 | Epic 2, 4 |
| FR5 | Token counting failures must not abort the request | Design §FR5 | Epic 1 |
| FR6 | Both native tools path and XML injection path must emit real counts | Design §FR6 | Epic 5 |

### Non-Functional Requirements

| ID | Requirement | Target | Epic |
|----|-------------|--------|------|
| NFR1 | Input token counting latency | < 10ms for typical requests | Epic 1 |
| NFR2 | Output token counting latency at stream end | < 5ms for typical responses | Epic 1 |
| NFR3 | Memory overhead for output accumulation | O(response_size) — same bytes as text already transmitted | Epic 4 |
| NFR4 | No regression in streaming latency (time to first token) | No measurable change | Epic 5 |

---

## File Changes Summary

| File | Change | Epic | Description |
|------|--------|------|-------------|
| `src/token_counter.rs` | Modified | Epic 1 | Add `count_tokens_for_request()` and `count_output_tokens()` |
| `src/copilot/types.rs` | Modified | Epic 2 | Add `usage: Option<Usage>` to `ChatCompletionChunk` |
| `src/anthropic/types.rs` | Modified | Epic 3 | Add `input_tokens: u32` parameter to `build_message_start_response()` |
| `src/streaming/state.rs` | Modified | Epic 4 | Add accumulation fields; update `new()`, `handle_text_delta()`, `handle_tool_call_delta()`, `process_chunk()`, `compute_output_tokens()`, `compute_input_tokens()`, `build_message_start()`, `handle_finish()`, `finalize()` |
| `src/handlers/messages.rs` | Modified | Epic 5 | Compute `input_tokens` in all streaming paths; pass to `StreamingState::new()` and `build_message_start_response()` |
| `tests/unit/token_counter_tests.rs` | Modified | Epic 6 | Unit tests for new helpers |
| `tests/unit/streaming_tests.rs` | Modified | Epic 6 | Unit tests for `StreamingState` token counting |
| `tests/unit/anthropic_types_tests.rs` | Modified | Epic 6 | Update existing tests + add new signature tests |
| `tests/integration/streaming_tests.rs` | Modified | Epic 6 | Integration tests for real usage values in SSE events |
| `CLAUDE.md` | Modified | Epic 7 | Update token counting and streaming state machine notes |
| `docs/known-issues.md` | Modified | Epic 7 | Remove zero token count entry |

---

## Testing Strategy

### Test Coverage

| Component | Unit Tests | Integration Tests | E2E Tests |
|-----------|------------|-------------------|-----------|
| `token_counter.rs` new helpers | Epic 6.1 | Epic 6.4 (consistency) | Manual |
| `StreamingState` input count | Epic 6.2 | Epic 6.4 | Manual |
| `StreamingState` output count | Epic 6.2 | Epic 6.4 | Manual |
| Upstream usage override | Epic 6.2, 6.3 | Epic 6.4 | — |
| XML injection paths | Epic 6.2 (indirect) | Epic 6.4 | Manual |

### Test Files

| File | Type | Coverage |
|------|------|----------|
| `tests/unit/token_counter_tests.rs` | Unit | New `count_tokens_for_request()` / `count_output_tokens()` |
| `tests/unit/streaming_tests.rs` | Unit | `StreamingState` token tracking and emission |
| `tests/unit/anthropic_types_tests.rs` | Unit | `build_message_start_response()` signature |
| `tests/integration/streaming_tests.rs` | Integration | End-to-end SSE usage field verification |
| `docs/development/e2e-testing.md` | Manual E2E | `/model` view header verification |

---

## Dependencies

### External Dependencies

No new external dependencies. `tiktoken-rs` is already in `Cargo.toml`.

### Internal Dependencies

| Module | Required By | Status |
|--------|-------------|--------|
| `src/token_counter.rs` | Epic 4, 5 | ✅ Exists |
| `src/anthropic/types.rs` (`AnthropicRequest`) | Epic 1 | ✅ Exists |
| `src/copilot/types.rs` (`Usage`) | Epic 2 | ✅ Exists |
| `src/streaming/state.rs` | Epic 4 | ✅ Exists |
| `src/handlers/messages.rs` | Epic 5 | ✅ Exists |

**Epic ordering constraint:** Epic 1 must complete before Epic 4 (state machine needs the helpers). Epic 2 must complete before Epic 4 (state machine reads `chunk.usage`). Epic 3 must complete before Epic 4 (state machine calls the updated function). Epic 4 must complete before Epic 5 (handler calls `StreamingState::new()` with new signature).

---

## Risk Assessment

| Risk | Impact | Probability | Mitigation | Epic |
|------|--------|-------------|------------|------|
| `cl100k_base()` initialization fails at runtime | High (counts silently stay 0) | Low | Both helpers return 0 on failure; not a regression vs. current hardcoded zeros | Epic 1 |
| `output_text` accumulation too large for very long responses | Medium | Low | Responses bounded by Copilot's `max_tokens`; typical max ~64K tokens ≈ ~256KB | Epic 4 |
| `handle_streaming()` / `handle_streaming_with_tools()` don't use `StreamingState` — more complex to fix | Medium | Confirmed | Task 5.2 specifically calls this out; read both functions before implementing | Epic 5 |
| Signature change to `build_message_start_response()` breaks call sites | Medium | Low | Compiler enforces update of all call sites; easy to find | Epic 3 |
| Truncated tool call path (`finish_reason: "length"`) emits wrong count | Low | Low | Task 4.7 explicitly specifies that only notice text (not discarded args) counts | Epic 4 |
| Tool argument accumulation double-counts if args overlap with text | Low | Very Low | Text and tool calls are mutually exclusive in practice | Epic 4 |

---

## Success Criteria

1. **Non-zero header** — After any successful streaming response, `/model` view shows `N/Xm tokens (P%)` with N > 0 (Epic 4, 5)
2. **Consistency** — `message_start.usage.input_tokens` equals the value returned by `POST /v1/messages/count_tokens` for the same request (Epic 1, 6)
3. **No regressions** — All existing unit and integration tests pass (Epic 6)
4. **Graceful degradation** — If tiktoken initialization fails, adapter still returns responses; usage fields fall back to 0 (Epic 1)
5. **Upstream override works** — If Copilot API returns usage in a chunk, those values appear in the SSE events (Epic 2, 4, 6)

---

## Rollout / Migration Plan

### Phase 1: Core infrastructure (Epics 1–3)
- [ ] Add token counting helpers to `token_counter.rs`
- [ ] Add `usage` field to `ChatCompletionChunk`
- [ ] Update `build_message_start_response()` signature
- [ ] Verify `cargo build` succeeds

### Phase 2: Streaming state machine (Epic 4)
- [ ] Update `StreamingState` struct and `new()`
- [ ] Implement accumulation in `handle_text_delta()` and `handle_tool_call_delta()`
- [ ] Implement upstream usage capture in `process_chunk()`
- [ ] Implement compute helpers and update emission points
- [ ] Verify `cargo build` succeeds

### Phase 3: Handler wiring (Epic 5)
- [ ] Wire `count_tokens_for_request()` in `handle_native_tools_streaming()`
- [ ] Wire counts in `handle_streaming()` and `handle_streaming_with_tools()`
- [ ] Verify `cargo build` succeeds

### Phase 4: Testing (Epic 6)
- [ ] Unit tests complete and passing
- [ ] Integration tests complete and passing
- [ ] Manual E2E: `/model` view shows non-zero counts

### Phase 5: Documentation and release (Epic 7)
- [ ] `CLAUDE.md` updated
- [ ] `docs/known-issues.md` updated
- [ ] All acceptance criteria met
- [ ] Final review
- [ ] Merge to main
- [ ] Archive design/plan docs to `docs/design/archive/`

---

## Epic Status Tracking

| Epic | Status | Start Date | End Date | Notes |
|------|--------|------------|----------|-------|
| Epic 1: Token counting helpers | DONE | 2026-04-06 | 2026-04-06 | |
| Epic 2: `ChatCompletionChunk.usage` | Not Started | - | - | |
| Epic 3: `build_message_start_response()` signature | Not Started | - | - | |
| Epic 4: `StreamingState` accumulation | Not Started | - | - | Depends on Epics 1–3 |
| Epic 5: Handler wiring | Not Started | - | - | Depends on Epic 4 |
| Epic 6: Testing | Not Started | - | - | Depends on Epics 4–5 |
| Epic 7: Documentation | Not Started | - | - | |

---

## Open Questions

| # | Question | Status | Blocker For |
|---|----------|--------|-------------|
| 1 | Does the Copilot API ever return `usage` in the final SSE chunk for non-Claude models (e.g., GPT-4o)? | Open — add field speculatively; verify with trace logs after deployment | Epic 2 |
| 2 | Do `handle_streaming()` and `handle_streaming_with_tools()` currently call `build_message_start_response()` directly, or do they emit raw JSON? | Open — requires reading full function bodies before implementing Task 5.2 | Epic 5 |
| 3 | Should the tiktoken encoder be cached in `AppState`? | Deferred — existing TODO in `token_counter.rs`; profile first | — |

---

## References

- [Design document](./USAGE-TOKEN-COUNTS.design.md)
- `src/streaming/state.rs` — current state machine with hardcoded zeros at lines 184 and 425
- `src/token_counter.rs` — existing `count_tokens()` implementation
- `src/anthropic/types.rs` — `build_message_start_response()` at line 942
- `src/copilot/types.rs` — `ChatCompletionChunk` struct (currently no `usage` field)
- `src/handlers/messages.rs` — streaming entry points: `handle_streaming()` (line 405), `handle_streaming_with_tools()` (line 623), `handle_native_tools_streaming()` (line 1138)
- `tests/unit/streaming_tests.rs` — existing streaming unit tests to extend
- `tests/integration/streaming_tests.rs` — existing streaming integration tests to extend
- LiteLLM `streaming_chunk_builder_utils.py` — reference implementation (`ChunkProcessor.calculate_usage()`)

---

## Notes

### Development Notes
- The three hardcoded zero locations to fix are explicitly listed in the design document's Appendix
- `handle_streaming_with_tools()` buffers all chunks before emitting any SSE — it does not use `StreamingState`. This path will need its own accumulation logic or refactoring to use `StreamingState`. This is the most uncertain part of the implementation.
- The `cl100k_base()` call is already made on every `count_tokens` endpoint request; adding it to streaming is consistent with current practice and not a new cost pattern.
- All new `pub` functions in `token_counter.rs` need to be exported correctly — check `src/lib.rs` if token counter is re-exported there.
