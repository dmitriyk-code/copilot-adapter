# Usage Token Counts in Streaming Responses — Design Document

**Status:** Proposed
**Date:** 2026-04-06
**Related:** `src/streaming/state.rs`, `src/token_counter.rs`, `src/anthropic/types.rs`, `src/copilot/types.rs`

---

## Executive Summary

Claude Code's `/model` view displays a token usage header (`N/1m tokens (P%)`) derived from the `usage` field in the Anthropic `message_start` SSE event. copilot-adapter currently hardcodes `input_tokens: 0` and `output_tokens: 0` in every streaming response because the GitHub Copilot API does not return token usage data in its SSE chunks. The result is that the header always shows `0/Nm tokens (0%)`, even after a response has been received, making context window tracking useless.

This document designs a local token-counting approach: count input tokens using tiktoken-rs before forwarding the request to the Copilot API (reusing existing `token_counter.rs` logic), and count output tokens by accumulating streamed text content as chunks arrive and running tiktoken-rs on the completed output at stream finalization. The computed values are then injected into `message_start` (`input_tokens`) and `message_delta` (`output_tokens`) SSE events respectively. This mirrors the approach used by LiteLLM for providers that don't return native usage data.

Key points:
- **No new dependencies** — `tiktoken-rs` is already in use for `count_tokens` endpoint
- **Minimal latency impact** — input counting is a single pass before the first upstream call; output counting accumulates incrementally during streaming and runs tiktoken once at stream end
- **Consistent with existing behaviour** — the same `cl100k_base` BPE encoder used for `POST /v1/messages/count_tokens` is used here, so estimated counts will be internally consistent

---

## Context / Background

### Current State

`StreamingState` in `src/streaming/state.rs` is the Axum SSE streaming state machine that translates OpenAI streaming chunks into Anthropic SSE events. It emits three events that carry token counts:

1. **`message_start`** — emitted on the first chunk. Built via `build_message_start_response()` in `src/anthropic/types.rs`. The `usage` field is hardcoded:
   ```rust
   usage: AnthropicUsage { input_tokens: 0, output_tokens: 0 }
   ```
   There is a comment: _"Usage starts at zero and content is empty"_.

2. **`message_delta`** (at stream end, from `handle_finish()`) — carries `output_tokens`:
   ```rust
   usage: MessageDeltaUsage { output_tokens: 0 }
   // TODO: wire actual token counts once ChatCompletionChunk exposes usage
   ```

3. **`message_delta`** (from `finalize()`, fallback path) — same hardcoded `output_tokens: 0`.

The GitHub Copilot API's SSE chunks (`ChatCompletionChunk` in `src/copilot/types.rs`) do **not** include a `usage` field at all — confirmed by trace log analysis. The struct even omits the field:
```rust
pub struct ChatCompletionChunk {
    pub id: String,
    pub object: String,
    pub created: i64,
    pub model: String,
    pub choices: Vec<ChunkChoice>,
    // no usage field
}
```

The existing `count_tokens` endpoint (`POST /v1/messages/count_tokens`) uses `tiktoken-rs` with `cl100k_base` and already counts input tokens accurately for arbitrary `AnthropicRequest`-shaped payloads. The logic lives in `src/token_counter.rs` and is fully reusable.

### Target State / Desired Behavior

After this change:
- The `message_start` event emits the real `input_tokens` count (estimated via tiktoken-rs on the full incoming Anthropic request — system prompt + messages + tools)
- The `message_delta` event at stream end emits the real `output_tokens` count (estimated via tiktoken-rs on the accumulated response text and/or tool call JSON)
- Claude Code's `/model` view displays accurate, non-zero token counts immediately after a response arrives
- The "Estimated usage by category" section (which already uses `count_tokens` independently) remains unchanged

---

## Problem Statement

**Observed behavior:**
- Claude Code's `/model` view shows `0/Nm tokens (0%)` in the header even after receiving responses
- `message_start.message.usage` always carries `{ "input_tokens": 0, "output_tokens": 0 }`
- `message_delta.usage` always carries `{ "output_tokens": 0 }`

**Expected behavior:**
- After a response is received, the header shows the actual approximate token count consumed
- `input_tokens` reflects the size of the request sent (system prompt + conversation history + tool definitions)
- `output_tokens` reflects the size of the response received (text content + tool calls)

**Impact:**
- All users see permanently broken context window tracking in the `/model` view header
- Users cannot judge how close they are to the context limit without switching to the "Estimated usage by category" breakdown
- The "Estimated usage by category" section works but only via a separate, proactive `count_tokens` call — not from live response data

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
| G6 | Works for non-streaming responses | Non-streaming `AnthropicResponse` also gets accurate usage filled in |

### Non-Goals

| ID | Non-Goal | Rationale |
|----|----------|-----------|
| NG1 | Exact token counts matching Copilot API's own counts | Copilot API doesn't expose them; tiktoken approximations are sufficient for UX |
| NG2 | Caching the tiktoken encoder in `AppState` | The existing `count_tokens` endpoint doesn't do this; defer until profiling shows need |
| NG3 | Per-model tokenizer selection (e.g. different BPE for Sonnet vs Opus) | All Copilot Claude models use the same underlying tokenizer; `cl100k_base` is accurate enough |
| NG4 | Counting cache creation or cache read tokens | Copilot API does not support prompt caching; these fields remain 0 |
| NG5 | Retroactively updating the "Estimated usage by category" section | That section uses its own independent `count_tokens` flow; it is already accurate |

---

## Research / Analysis

### Key Findings

1. **GitHub Copilot API never returns usage in SSE chunks.** Trace log analysis of `logs4.txt` confirms every `ChatCompletionChunk` contains only `id`, `object`, `created`, `model`, and `choices[].delta`. No `usage` field was ever observed, even in the final `finish_reason: "stop"` chunk.

2. **LiteLLM solves this identically.** LiteLLM's streaming handler (`streaming_handler.py`) accumulates all streamed chunks, then on stream end calls `token_counter(model, messages=messages)` for prompt tokens and `token_counter(model, text=output)` for completion tokens. These are injected into the final response. The tokenizer is `tiktoken` with `cl100k_base`.

3. **`token_counter.rs` already handles input counting correctly.** The existing `count_tokens()` function in `src/token_counter.rs` accepts a `CountTokensRequest` (system + messages + tools) and returns an accurate `cl100k_base` estimate. `AnthropicRequest` has the same fields and can be trivially converted.

4. **`StreamingState` is the right place for output counting.** The state machine owns every text and tool-call fragment. It already accumulates tool-use content in a buffer (`tool_use_buffer`). The natural place to count output tokens is at finalization, after all content has been seen.

5. **`build_message_start_response()` needs to accept an `input_tokens` argument.** Currently it is a zero-argument constructor. Adding an `input_tokens: u32` parameter is the minimal change needed.

6. **`ChatCompletionChunk` could theoretically gain a `usage` field in the future.** The struct should be updated to include an optional `usage: Option<Usage>` field so the adapter can use real counts if the API starts returning them, falling back to tiktoken if not.

### Options Considered

#### Option A: Count at `StreamingState` construction (Recommended)

Count input tokens in the messages handler before entering the streaming state machine, then pass the count into `StreamingState::new()`. Output tokens are accumulated inside `StreamingState` as chunks arrive, finalized at `finalize()`.

**Pros:**
- Input counting is a single pass, complete before the first SSE chunk is emitted
- Output counting is incremental — O(1) per chunk, one BPE pass at end
- `StreamingState` becomes self-contained: it receives the input count and computes the output count
- Clean separation of concerns — counting is done before/during streaming, not after

**Cons:**
- `StreamingState::new()` signature changes (gains `input_tokens: u32`)
- Requires passing `AnthropicRequest` (or a pre-counted value) through the handler call chain

#### Option B: Count at finalization only

After the stream ends, count both input tokens (from the request) and output tokens (from accumulated text), then inject both into the final `message_delta`.

**Pros:**
- Simpler — no signature changes to `build_message_start_response()`

**Cons:**
- `message_start` still emits `input_tokens: 0` — Claude Code uses `message_start` as the source of truth for input tokens in the header
- Fundamentally broken for the display use-case: the header shows `0` until the entire response is received, then still shows `0` because `message_start` can't be retroactively updated in SSE

#### Option C: Add `usage` field to `ChatCompletionChunk` and wait for Copilot API

Update the struct to accept usage, parse it when present, use tiktoken as fallback.

**Pros:**
- Future-proof: if Copilot API ever returns usage, the adapter picks it up automatically

**Cons:**
- Does not solve the current problem — Copilot API does not return usage today
- Should be done alongside Option A as a bonus, not instead of it

### Recommended Approach

**Option A** as the primary implementation, **plus** the `ChatCompletionChunk` usage field from Option C as a forward-compatibility measure. If the upstream chunk contains usage, use it; otherwise fall back to the tiktoken estimate.

---

## Proposed Design / Architecture

### Component Overview

```
Claude Code  →  POST /v1/messages  →  messages handler
                                            │
                              ┌─────────────▼──────────────┐
                              │  count_input_tokens()       │
                              │  (token_counter::count)     │
                              │  → input_tokens: u32        │
                              └─────────────┬──────────────┘
                                            │
                              ┌─────────────▼──────────────┐
                              │  StreamingState::new(       │
                              │    name_mapping,            │
                              │    input_tokens,            │
                              │  )                          │
                              └─────────────┬──────────────┘
                                            │
                              ┌─────────────▼──────────────────────────────┐
                              │  process_chunk() × N                        │
                              │  - first chunk → message_start              │
                              │      usage.input_tokens = input_tokens      │
                              │  - text delta → accumulate output_text      │
                              │  - tool delta → accumulate tool_json        │
                              └─────────────┬──────────────────────────────┘
                                            │
                              ┌─────────────▼──────────────────────────────┐
                              │  finalize() / handle_finish()               │
                              │  - count_output_tokens(output_text +        │
                              │      tool_json) → output_tokens: u32        │
                              │  - message_delta.usage.output_tokens        │
                              │      = output_tokens                        │
                              └────────────────────────────────────────────┘
                                            │
                              ┌─────────────▼──────────────────────────────┐
                              │  ChatCompletionChunk.usage (if present)     │
                              │  → override tiktoken estimate               │
                              └────────────────────────────────────────────┘
```

### Technical Details

#### 1. `src/copilot/types.rs` — Add optional `usage` to `ChatCompletionChunk`

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
    /// Token usage statistics.  Most providers (including GitHub Copilot) omit
    /// this field from streaming chunks; when present it takes precedence over
    /// the local tiktoken estimate.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub usage: Option<Usage>,
}
```

#### 2. `src/token_counter.rs` — Add `count_tokens_for_request()` helper

A thin wrapper that takes `&AnthropicRequest` instead of `&CountTokensRequest`, avoiding duplication of the field mapping:

```rust
/// Count input tokens for an AnthropicRequest (system + messages + tools).
///
/// Reuses the same `cl100k_base` BPE encoder as the `count_tokens` endpoint.
/// Returns 0 on encoder failure rather than propagating the error — token
/// counting is best-effort and must not block the request.
pub fn count_tokens_for_request(request: &AnthropicRequest) -> u32 {
    let count_request = CountTokensRequest {
        model: request.model.clone(),
        messages: request.messages.clone(),
        system: request.system.clone(),
        tools: request.tools.clone(),
    };
    count_tokens(&count_request).unwrap_or(0)
}

/// Count output tokens for a completed response text.
///
/// Tokenizes the full accumulated output using `cl100k_base`.
/// Returns 0 on encoder failure.
pub fn count_output_tokens(text: &str) -> u32 {
    match cl100k_base() {
        Ok(bpe) => bpe.encode_with_special_tokens(text).len() as u32,
        Err(_) => 0,
    }
}
```

#### 3. `src/anthropic/types.rs` — Update `build_message_start_response()`

Add `input_tokens` parameter:

```rust
/// Build the initial `AnthropicResponse` shell used in the `message_start`
/// streaming event.  `input_tokens` is the tiktoken estimate for the request;
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

#### 4. `src/streaming/state.rs` — Track input tokens, accumulate output, emit real counts

Key changes:

```rust
pub struct StreamingState {
    // ... existing fields ...

    /// Pre-computed input token count from the Anthropic request.
    /// Injected into the `message_start` event.
    input_tokens: u32,

    /// Accumulated output text for token counting at stream end.
    /// Text content deltas are appended here as they arrive.
    output_text: String,

    /// Accumulated tool call JSON for token counting at stream end.
    /// Argument fragments from tool call deltas are appended here.
    output_tool_json: String,
}

impl StreamingState {
    pub fn new(name_mapping: HashMap<String, String>, input_tokens: u32) -> Self {
        Self {
            // ... existing fields ...
            input_tokens,
            output_text: String::new(),
            output_tool_json: String::new(),
        }
    }
```

In `handle_text_delta()`, append text to the accumulator:
```rust
    // Accumulate for output token counting.
    self.output_text.push_str(text);
```

In `handle_tool_call_delta()`, append argument fragments to the accumulator:
```rust
    if let Some(args) = &func.arguments {
        if !args.is_empty() {
            self.output_tool_json.push_str(args);
        }
    }
```

In `handle_finish()` and `finalize()`, compute and emit real `output_tokens`:
```rust
fn compute_output_tokens(&self) -> u32 {
    // If the upstream chunk carried usage, prefer it (Option C forward-compat).
    // Otherwise use the accumulated tiktoken estimate.
    let combined = format!("{}{}", self.output_text, self.output_tool_json);
    token_counter::count_output_tokens(&combined)
}
```

The `handle_finish()` method replaces the hardcoded `output_tokens: 0`:
```rust
events.push(StreamEvent::MessageDelta {
    delta: MessageDeltaBody { stop_reason, stop_sequence: None },
    usage: MessageDeltaUsage {
        output_tokens: self.compute_output_tokens(),
    },
});
```

The `finalize()` fallback path similarly uses `self.compute_output_tokens()`.

In `build_message_start()`:
```rust
fn build_message_start(&self) -> StreamEvent {
    StreamEvent::MessageStart {
        message: build_message_start_response(
            self.message_id.as_deref().unwrap_or("unknown"),
            self.model.as_deref().unwrap_or("unknown"),
            self.input_tokens,
        ),
    }
}
```

#### 5. `src/handlers/messages.rs` — Compute input tokens before entering streaming

Both the native tools path and the XML injection path call into streaming. Both must compute `input_tokens` before constructing `StreamingState`.

For the streaming path:
```rust
// Compute input token estimate before streaming begins.
let input_tokens = token_counter::count_tokens_for_request(&request);

// ... existing code to build openai_request ...

let mut state_machine = StreamingState::new(name_mapping, input_tokens);
```

For the non-streaming path, the existing non-streaming `AnthropicResponse` is built from `ChatCompletion::to_anthropic_response()`. That function already handles `usage` from the Copilot response body. However, non-streaming Copilot responses also include `usage` in the response body (confirmed in `src/copilot/types.rs` — `ChatCompletion` has `pub usage: Option<Usage>`). So non-streaming is already handled correctly if the API returns it; we add a fallback for when it returns `null`:

```rust
// In to_anthropic_response() in src/anthropic/types.rs:
let usage = self.usage
    .as_ref()
    .map(|u| AnthropicUsage {
        input_tokens: u.prompt_tokens,
        output_tokens: u.completion_tokens,
    })
    .unwrap_or_else(|| AnthropicUsage {
        // Fallback: count locally if the API omitted usage.
        input_tokens: 0,   // Not available here without the original request
        output_tokens: 0,
    });
```

For non-streaming, `to_anthropic_response()` is called deep inside the Copilot client. Passing the original request in is a larger refactor. Since non-streaming responses from Copilot *do* include `usage` in the body (`ChatCompletion.usage`), the existing fallback to zeros is only hit in error cases. **Non-streaming is deferred as a follow-up** — the primary user-visible issue is the streaming path.

#### 6. Upstream usage override (Option C forward-compat)

If GitHub Copilot ever starts returning `usage` in streaming chunks, `ChatCompletionChunk.usage` will now be deserialized. `StreamingState` can check for it in the final chunk (the one with `finish_reason`) and prefer it over the tiktoken estimate:

```rust
// In process_chunk():
if let Some(upstream_usage) = &chunk.usage {
    // Real usage data from upstream — store it.
    self.upstream_output_tokens = Some(upstream_usage.completion_tokens);
    self.upstream_input_tokens = Some(upstream_usage.prompt_tokens);
}
```

And in `compute_output_tokens()`:
```rust
fn compute_output_tokens(&self) -> u32 {
    // Prefer upstream usage if available.
    if let Some(tokens) = self.upstream_output_tokens {
        return tokens;
    }
    let combined = format!("{}{}", self.output_text, self.output_tool_json);
    token_counter::count_output_tokens(&combined)
}
```

And `build_message_start()` similarly prefers `self.upstream_input_tokens` over `self.input_tokens`.

---

## Requirements

### Functional Requirements

| ID | Requirement | Source |
|----|-------------|--------|
| FR1 | `message_start.message.usage.input_tokens` must be non-zero for any non-empty request | User bug report: `/model` shows `0` |
| FR2 | `message_delta.usage.output_tokens` must be non-zero after a text response | User bug report: `/model` shows `0` |
| FR3 | Counts must be consistent with those returned by `POST /v1/messages/count_tokens` | Internal consistency requirement |
| FR4 | If the Copilot API returns usage in a chunk, it must take precedence over tiktoken | Forward-compatibility |
| FR5 | Token counting failures must not abort the request | Degraded accuracy is acceptable; broken requests are not |
| FR6 | Both native tools path and XML injection path must emit real counts | Both paths use `StreamingState` |

### Non-Functional Requirements

| ID | Requirement | Target |
|----|-------------|--------|
| NFR1 | Input token counting latency | < 10ms for typical requests (same target as `count_tokens` endpoint) |
| NFR2 | Output token counting latency at stream end | < 5ms for typical responses (single BPE pass over accumulated text) |
| NFR3 | Memory overhead for output accumulation | O(response_size) — same bytes as the text already transmitted |
| NFR4 | No regression in streaming latency (time to first token) | Input counting completes before streaming begins; should be undetectable |

---

## File Changes Summary

| File | Change | Description |
|------|--------|-------------|
| `src/copilot/types.rs` | Modified | Add `usage: Option<Usage>` field to `ChatCompletionChunk` |
| `src/token_counter.rs` | Modified | Add `count_tokens_for_request()` and `count_output_tokens()` helpers |
| `src/anthropic/types.rs` | Modified | Add `input_tokens: u32` parameter to `build_message_start_response()` |
| `src/streaming/state.rs` | Modified | Add `input_tokens`, `output_text`, `output_tool_json`, optional upstream usage fields; update `new()`, `handle_text_delta()`, `handle_tool_call_delta()`, `handle_finish()`, `finalize()`, `build_message_start()` |
| `src/handlers/messages.rs` | Modified | Compute `input_tokens` before streaming; pass to `StreamingState::new()` (both native tools and XML injection paths) |
| `tests/unit/` | Modified | Add/update unit tests for new token counting functions and streaming state |

---

## Testing Strategy

### Unit Tests

1. **`src/token_counter.rs` — new helpers:**
   - `count_tokens_for_request()` returns same value as `count_tokens()` for equivalent requests
   - `count_output_tokens("")` returns 0
   - `count_output_tokens("Hello, world!")` returns > 0

2. **`src/streaming/state.rs` — input token propagation:**
   - `StreamingState::new(HashMap::new(), 42)` produces a `message_start` with `input_tokens: 42` on the first chunk
   - `StreamingState::new(HashMap::new(), 0)` produces `input_tokens: 0` (no regression)

3. **`src/streaming/state.rs` — output token accumulation:**
   - After processing multiple text chunks, `finalize()` emits `message_delta.usage.output_tokens > 0`
   - After processing a tool call chunk, `finalize()` includes tool JSON in the count
   - After a `finish_reason: "length"` truncation, `output_tokens` reflects only emitted content (not discarded tool call)
   - `output_tokens` increases with response length (longer text → higher count)

4. **`src/streaming/state.rs` — upstream usage override:**
   - When a chunk carries `usage: { prompt_tokens: 100, completion_tokens: 50 }`, `message_start` emits `input_tokens: 100` and `message_delta` emits `output_tokens: 50`
   - When upstream usage is absent, tiktoken estimate is used

5. **`src/anthropic/types.rs` — `build_message_start_response()`:**
   - Returns `usage.input_tokens` matching the passed-in value
   - `output_tokens` is always 0 in the `message_start` response

### Integration Tests

1. **Streaming response produces non-zero usage:**
   - Setup: Start adapter, send a streaming chat request
   - Action: Collect all SSE events
   - Verification: `message_start.message.usage.input_tokens > 0`; `message_delta.usage.output_tokens > 0`

2. **Input count consistency with `count_tokens` endpoint:**
   - Setup: Send identical request body to both `POST /v1/messages/count_tokens` and `POST /v1/messages`
   - Action: Compare `count_tokens` response vs `message_start.usage.input_tokens`
   - Verification: Values are equal (same tiktoken path)

3. **Tool call responses produce non-zero output tokens:**
   - Setup: Send request with tools, model performs a tool call
   - Action: Collect SSE events
   - Verification: `message_delta.usage.output_tokens > 0` even when response is a tool_use block

### Manual E2E Tests

1. **`/model` view shows non-zero counts:**
   - Start adapter, start Claude Code connected to it
   - Select Opus 4.6 1M model
   - Send a message, wait for response
   - Open `/model` view
   - Expected: Header shows `N/1m tokens (P%)` with N > 0

2. **Counts increase over a multi-turn conversation:**
   - Send several messages in the same session
   - After each response, open `/model` view
   - Expected: Token count in header increases with each turn

---

## Risk Assessment

| Risk | Impact | Probability | Mitigation |
|------|--------|-------------|------------|
| tiktoken-rs `cl100k_base()` initialization fails | High (counting returns 0 silently) | Low | Return 0 on error; already the pattern in existing code |
| Output text accumulation allocates too much memory for very long responses | Medium | Low | Responses are bounded by Copilot's `max_tokens`; typical max is 64K tokens ≈ ~256KB text |
| `input_tokens` count diverges from Copilot API's actual count | Low (UX inaccuracy only) | Medium | Acceptable — display accuracy is the goal, not billing accuracy |
| Breaking change to `StreamingState::new()` / `build_message_start_response()` call sites | Medium | Low | Both are internal; all callers are in this codebase; compiler enforces updates |
| Non-streaming responses still show 0 input_tokens in failure cases | Low | Medium | Non-streaming Copilot responses already include `usage`; fallback to 0 only on API error |
| Tool call JSON accumulation double-counts tokens if args overlap with text | Low | Very Low | Tool calls and text are mutually exclusive in practice; worst case is slight overcount |

---

## Success Criteria

1. **Non-zero header** — After any successful streaming response, `/model` view shows `N/Nm tokens (P%)` with N > 0
2. **Consistency** — `message_start.usage.input_tokens` equals the value returned by `POST /v1/messages/count_tokens` for the same request
3. **No regressions** — All existing unit and integration tests pass
4. **Graceful degradation** — If tiktoken initialization fails, the adapter still returns responses; usage fields fall back to 0

---

## Performance / Metrics

| Metric | Target | How to Measure |
|--------|--------|----------------|
| Input token counting latency | < 10ms | Benchmark `count_tokens_for_request()` on a typical 28KB system prompt + 10 messages |
| Output token counting latency | < 5ms | Benchmark `count_output_tokens()` on a 2000-token response |
| Memory overhead per request | < 300KB additional | Measure peak heap during a long streaming response (Valgrind/heaptrack) |
| Time to first SSE byte (regression check) | No measurable change | Compare P99 latency before/after with trace logging enabled |

---

## Design Decisions

| Decision | Rationale |
|----------|-----------|
| Count input tokens in the messages handler, not inside `StreamingState` | The handler has direct access to `AnthropicRequest`; passing a pre-counted `u32` into `StreamingState` is simpler than passing the full request |
| Accumulate output text as `String` in `StreamingState` | Minimal overhead; no structural change to the streaming loop; consistent with LiteLLM's approach |
| Use `cl100k_base` for output counting | Same encoder as `count_tokens` endpoint; internally consistent; avoids model-specific selection complexity |
| Return 0 on token counting error, never propagate | Token counting is best-effort; failing it must not degrade the user's actual conversation |
| Defer non-streaming fallback | Non-streaming Copilot responses include `usage` in the body; zero-fallback only hits error cases; not worth the refactor now |
| Add `usage` field to `ChatCompletionChunk` for forward-compatibility | Zero cost to add; if Copilot API starts returning streaming usage (as some OpenAI-compatible providers do), the adapter picks it up automatically |

---

## Open Questions

| # | Question | Status |
|---|----------|--------|
| 1 | Does the Copilot API ever return `usage` in the final SSE chunk (e.g. for non-Claude models like GPT-4o)? | Open — add the field speculatively; verify with trace logs after deployment |
| 2 | Should the tiktoken encoder be cached in `AppState` to avoid repeated BPE vocab loading? | Deferred — existing TODO in `token_counter.rs` already notes this; profile first |
| 3 | Should `output_tokens` be emitted progressively (estimated count growing in each `content_block_delta`) or only at finalization? | Resolved — finalization only; Anthropic protocol puts `output_tokens` in `message_delta`, not per-chunk |
| 4 | Should tool call arguments be counted separately as `output_tokens` or omitted? | Resolved — include them; Claude Code uses `output_tokens` for total context tracking, and tool calls consume real output capacity |

---

## Documentation Updates

### Files to Update

| File | Changes |
|------|---------|
| `CLAUDE.md` | Update "Token counting" note to describe that streaming responses now carry real `input_tokens` and `output_tokens`; remove the implicit "always zero" implication |
| `CLAUDE.md` | Update "Streaming state machine" note to mention output text accumulation and tiktoken counting at finalization |
| `docs/known-issues.md` | Remove or update any entry about `/model` view showing zero token counts |

---

## Verification Steps

1. **Build succeeds:**
   ```bash
   cargo build
   ```
   Expected: No compiler errors; all `build_message_start_response()` call sites updated.

2. **Unit tests pass:**
   ```bash
   cargo test --test unit
   ```
   Expected: All existing tests pass; new token counting tests pass.

3. **Integration tests pass:**
   ```bash
   cargo test --test integration
   ```
   Expected: All existing tests pass; new streaming usage test passes.

4. **Manual trace log verification:**
   Start the adapter with `--log-level trace`, send a request, inspect logs for:
   ```
   message_start ... "usage": { "input_tokens": <N>, "output_tokens": 0 }
   message_delta ... "usage": { "output_tokens": <M> }
   ```
   Both N and M must be > 0 for a non-trivial request.

5. **Claude Code `/model` view:**
   Open `/model` view after a response; confirm header shows `N/Xm tokens (P%)` with N > 0.

---

## References

- `src/streaming/state.rs` — current streaming state machine with `output_tokens: 0` TODO
- `src/token_counter.rs` — existing tiktoken-rs BPE counting logic
- `src/anthropic/types.rs` — `build_message_start_response()`, `AnthropicUsage`, `MessageDeltaUsage`
- `src/copilot/types.rs` — `ChatCompletionChunk` (currently missing `usage` field)
- `src/handlers/messages.rs` — entry points for both native tools and XML injection streaming paths
- LiteLLM `litellm/litellm_core_utils/streaming_handler.py` — reference implementation of streaming token injection
- LiteLLM `litellm/litellm_core_utils/streaming_chunk_builder_utils.py` — `ChunkProcessor.calculate_usage()` reference
- Anthropic SSE protocol: `message_start` carries `input_tokens`; `message_delta` carries `output_tokens`
- `logs4.txt` / `conversation_logs5.txt` — trace log evidence confirming Copilot API never returns usage in SSE chunks

---

## Appendix

### Anthropic SSE event sequence with usage fields

```
event: message_start
data: {"type":"message_start","message":{"id":"msg_...","type":"message","role":"assistant",
       "content":[],"model":"claude-opus-4.6","stop_reason":null,"stop_sequence":null,
       "usage":{"input_tokens": 1842, "output_tokens": 0}}}     ← input_tokens here

event: content_block_start
data: {"type":"content_block_start","index":0,"content_block":{"type":"text","text":""}}

event: content_block_delta
data: {"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"Hello"}}

... more deltas ...

event: content_block_stop
data: {"type":"content_block_stop","index":0}

event: message_delta
data: {"type":"message_delta","delta":{"stop_reason":"end_turn","stop_sequence":null},
       "usage":{"output_tokens": 47}}                           ← output_tokens here

event: message_stop
data: {"type":"message_stop"}
```

### LiteLLM reference: streaming token injection (Python)

```python
# litellm/litellm_core_utils/streaming_chunk_builder_utils.py
# ChunkProcessor.calculate_usage() (simplified)

if prompt_tokens is None:
    prompt_tokens = token_counter(model=model, messages=messages)

if completion_tokens is None:
    completion_tokens = token_counter(
        model=model,
        text=completion_output,
        count_response_tokens=True,
    )

usage = Usage(
    prompt_tokens=prompt_tokens,
    completion_tokens=completion_tokens,
    total_tokens=prompt_tokens + completion_tokens,
)
```

### Current hardcoded zero locations (to be fixed)

| Location | Code | Fix |
|----------|------|-----|
| `src/anthropic/types.rs:952` | `usage: AnthropicUsage { input_tokens: 0, output_tokens: 0 }` in `build_message_start_response()` | Accept `input_tokens` parameter |
| `src/streaming/state.rs:184` | `usage: MessageDeltaUsage { output_tokens: 0 }` in `finalize()` | Use `self.compute_output_tokens()` |
| `src/streaming/state.rs:425` | `usage: MessageDeltaUsage { output_tokens: 0 }` in `handle_finish()` with TODO comment | Use `self.compute_output_tokens()` |
