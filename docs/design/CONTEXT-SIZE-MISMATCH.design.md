# Context Size Mismatch — Design Document

**Status:** Draft
**Date:** 2026-04-09
**Severity:** High
**Related:** `docs/design/CONTEXT-WINDOW-AND-TRUNCATION.design.md`

> **⚠️ Partially superseded (2026-06):** Failure 1 in this document (1M model
> activation via a `-1m` model-name suffix) no longer applies. GitHub Copilot
> consolidated its Claude SKUs — there are no `-1m` model IDs and the base
> models are 1M-native, so the adapter no longer appends `-1m`. The diagnostic
> header logging (Fix 1) and the `prompt_too_long` error-translation robustness
> (Fix 2) remain valid. See
> `docs/design/COPILOT-1M-MODEL-CONSOLIDATION.design.md`.

---

## Executive Summary

When a Claude Code user selects the "Opus 4.6 1M" model variant, the copilot-adapter fails to select the corresponding 1M-context model on the Copilot API. Instead, it sends requests to the standard `claude-opus-4.6` model (168K context). As the conversation grows beyond 168K tokens, the Copilot API rejects it with `model_max_prompt_tokens_exceeded`. The adapter returns this as HTTP 502 (Bad Gateway) instead of translating it to the Anthropic `prompt_too_long` format. Claude Code then retries the request 10 times (502 is a 5xx error), all of which fail identically, and the session becomes permanently stuck.

Key points:
- The `anthropic-beta: context-1m-*` header is not being received by the adapter for unknown reasons — the 1M model variant is never activated. The Anthropic SDK **always** sends `betas` as an HTTP header (confirmed via SDK source and protocol spy logs), so the root cause is likely in a specific Claude Code code path that skips the header (e.g., sub-agent queries, API provider configuration)
- A secondary bug in `parse_prompt_too_long` causes the error to be returned as 502 instead of 400 — disabling Claude Code's automatic context compaction recovery
- Even if the `prompt_too_long` error were correctly translated, Claude Code's autocompact would not trigger proactively because it believes it has ~820K free tokens (computed against the 1M window)
- Multiple cascading failures turn a recoverable situation into a permanent dead-end

---

## Context / Background

### Current State

The copilot-adapter translates Anthropic API requests to the GitHub Copilot API format. When Claude Code selects a 1M-context model variant, it signals this via the `anthropic-beta: context-1m-*` HTTP header. The adapter's `has_1m_context_beta()` function detects this header and appends `-1m` to the normalized Copilot model name (e.g., `claude-opus-4.6` → `claude-opus-4.6-1m`).

The adapter also has a `parse_prompt_too_long()` function that detects Copilot API errors of type `model_max_prompt_tokens_exceeded` and translates them to the Anthropic `prompt_too_long` format, which triggers Claude Code's automatic context compaction.

### Target State / Desired Behavior

1. When the user selects "Opus 4.6 1M", the adapter should use the `claude-opus-4.6-1m` Copilot model
2. When the conversation exceeds the model's context limit, the adapter should return HTTP 400 with `"code": "prompt_too_long"` and the specific message format `"prompt is too long: N tokens > M maximum"` that triggers Claude Code's regex-based compaction
3. Claude Code should be able to recover from context overflow via automatic compaction

---

## Problem Statement

**Observed behavior:**
- User selects `/model Opus 4.6 1M` in Claude Code
- Claude Code displays "Opus 4.6 (1M context)" with 1M token budget in context usage
- After a long conversation (~445 messages, ~168K tokens), the Copilot API rejects requests with `prompt token count of 168178 exceeds the limit of 168000`
- The adapter returns HTTP 502 (Bad Gateway) to Claude Code
- Claude Code retries 10 times (treating 502 as transient server error), all fail
- The session becomes permanently stuck — no further progress is possible

**Expected behavior:**
- The adapter should use the 1M-context model variant when signaled by Claude Code
- If the prompt exceeds the limit, the adapter should return HTTP 400 with `prompt_too_long`, enabling Claude Code's automatic compaction
- Claude Code should proactively compact the conversation before reaching the model's actual limit

**Impact:**
- Any user with a long conversation (>168K tokens) using the 1M model variant through the adapter will hit this wall
- The session is unrecoverable without manual `/compact` or `/clear`
- Silent failure — no clear error message indicates what went wrong

---

## Goals and Non-Goals

### Goals

| ID | Goal | Success Criteria |
|----|------|------------------|
| G1 | 1M context model correctly activated | When Claude Code sends `anthropic-beta: context-1m-*`, the adapter selects the `-1m` Copilot model variant |
| G2 | `prompt_too_long` error correctly translated | Copilot API's `model_max_prompt_tokens_exceeded` returns HTTP 400 with Anthropic-format `prompt_too_long` to Claude Code |
| G3 | Diagnostic logging for context-1m detection | Log whether the `anthropic-beta` header is present and its value, for debugging |
| G4 | ~~Removed~~ | ~~Fallback: accept `betas` in request body~~ — ruled out; SDK always sends `betas` as HTTP header, never in body |

### Non-Goals

| ID | Non-Goal | Rationale |
|----|----------|-----------|
| NG1 | Implementing proactive context compaction in the adapter | This is Claude Code's responsibility; the adapter just needs correct error translation |
| NG2 | Fixing Claude Code's model/agent inheritance | Out of scope; Claude Code's agent model selection is its own concern |

---

## Research / Analysis

### Log Analysis

**Adapter logs** (`logs.txt`, 512KB, trace-level) and **conversation logs** (`conversation-logs.txt`) from a real session were analyzed.

#### Timeline of Events

1. **00:55:06** — First request: `model=claude-opus-4-6`, 437 messages, `stream=true`. 26 tools. Succeeded after 8.6s.
2. **00:55:16** — Second request: 439 messages. Succeeded.
3. **00:55:20** — Third request: 441 messages. Succeeded.
4. **00:55:26** — Fourth request: 443 messages. Succeeded.
5. **00:55:31** — Fifth request: 445 messages. **FAILED**: `prompt token count of 168178 exceeds the limit of 168000`. Returned as HTTP 502.
6. **00:55:32 – 00:58:06** — Claude Code retried 9 more times (10 total), all identical 502 failures.

#### Key Evidence

1. **Model name**: All requests used `model=claude-opus-4-6` (incoming) → `model=claude-opus-4.6` (outgoing). Never `claude-opus-4.6-1m`.

2. **No `context-1m` detection**: The log message `"1M context beta detected, selecting Copilot 1M model variant"` (emitted at `src/handlers/messages.rs:164`) never appears.

3. **Claude Code context display** (captured in system prompt context output embedded in messages):
   ```
   Opus 4.6 (1M context)
   claude-opus-4-6[1m]
   0/1m tokens (0%)
   System prompt: 6k tokens (0.6%)
   System tools: 14.3k tokens (1.4%)
   Messages: 136.4k tokens (13.6%)
   Free space: 821.8k (82.2%)
   ```
   Claude Code believes it has 82% free context (computed against 1M), but the actual model only has 168K.

4. **Error translation failure**: `parse_prompt_too_long` returns `None` despite the body matching the expected format. The error is wrapped as `CopilotError` (HTTP 502) instead of `PromptTooLong` (HTTP 400).

5. **Retry storm**: Claude Code retries 502 errors up to 10 times with exponential backoff. Each retry sends the identical 445-message payload, which always exceeds 168K.

### Root Cause Analysis

There are **three cascading failures**:

#### Failure 1: 1M Model Not Activated (Primary)

The `anthropic-beta: context-1m-*` HTTP header is not detected by the adapter. Two possible sub-causes:

**~~Theory A: SDK sends `betas` in request body, not HTTP header~~ — RULED OUT**
- Investigation of the Anthropic SDK source and protocol spy logs confirms the SDK **always** sends `betas` as the `anthropic-beta` HTTP header, never as a JSON body field, regardless of `ANTHROPIC_BASE_URL`. Protocol spy logs from a real session show:
  ```
  POST /v1/messages
  Headers:
    anthropic-beta: claude-code-20250219,oauth-2025-04-20,context-1m-2025-08-07,...
  Body:
    { "model": "claude-opus-4-6", ... }  // NO "betas" field
  ```

**Theory B: SDK sends the header but adapter doesn't detect it**
- The adapter has no logging of incoming HTTP headers at all — only the `has_1m_context_beta()` result is indirectly observable via the "1M context beta detected" log.
- A bug in `has_1m_context_beta()` or in axum's header extraction could cause the header to be missed.
- **Evidence against**: The function has comprehensive unit tests covering comma-separated values, multiple headers, prefix matching, etc. axum/hyper header handling is well-tested.
- **Verdict**: Unlikely but cannot be ruled out without diagnostic logging.

**Theory C: Claude Code doesn't send the header in this specific code path (Most Likely)**
- Claude Code normalizes the model name by stripping `[1m]`: `claude-opus-4-6[1m]` → `claude-opus-4-6`.
- The `betas` array (including `context-1m-2025-08-07`) is passed to `anthropic.beta.messages.create()`.
- The SDK converts this to the `anthropic-beta` HTTP header.
- However, there may be a specific code path (e.g., sub-agent query, API provider configuration, or a code path that uses `messages.create()` instead of `beta.messages.create()`) where betas are not included.
- The protocol spy logs showing the header present may be from a different code path than the one that triggered the failure.
- **Verdict**: Most likely explanation. Diagnostic logging in the adapter will confirm.

**Recommendation**: Add diagnostic logging of incoming `anthropic-beta` headers to determine whether the header is absent (Theory C) or present but missed (Theory B).

#### Failure 2: `prompt_too_long` Error Not Translated (Secondary)

The `parse_prompt_too_long()` function returns `None` for the body:
```json
{"error":{"message":"prompt token count of 168178 exceeds the limit of 168000","code":"model_max_prompt_tokens_exceeded"}}
```

This body matches the expected format exactly, and the function has unit tests passing with identical input. The function is exercised on every 400 error in the streaming path (`stream_chat_completion` → `handle_error_response`).

**Possible sub-causes:**
- Invisible characters in the response body (BOM, zero-width spaces)
- Response body encoding issues (the streaming request uses `Accept: text/event-stream`, and the error response may have unusual encoding)
- The response body may be wrapped in SSE framing (e.g., `data: {...}\n\n`) that the tracing log doesn't show

**Impact**: Without correct translation, the adapter returns HTTP 502 (`CopilotError` → `BAD_GATEWAY`) instead of HTTP 400 (`PromptTooLong`). Claude Code treats 502 as a transient server error and retries 10 times, instead of triggering context compaction.

#### Failure 3: Autocompact Never Triggers (Contributing)

Claude Code's autocompact threshold is computed as:
```
effective_window = context_window - reserved_for_summary
autocompact_threshold = effective_window - AUTOCOMPACT_BUFFER_TOKENS

For 1M model:
  effective_window = 1,000,000 - 20,000 = 980,000
  autocompact_threshold = 980,000 - 13,000 = 967,000
```

With ~157K tokens used (messages + tools + system), Claude Code sees 82% free space and never triggers autocompact. The actual model limit (168K) is far below the autocompact threshold (967K).

Even if the `prompt_too_long` error were correctly returned, Claude Code's reactive compaction would need to compact enough to fit within 168K — a massive reduction from its believed 1M capacity. The token gap (178 tokens over 168K) is small, but the fundamental mismatch between believed context (1M) and actual context (168K) means compaction would likely succeed once but fail again quickly as the conversation grows.

### Options Considered

#### ~~Option A: Add `betas` field to `AnthropicRequest`~~ — Ruled Out

**Ruled out** after confirming the Anthropic SDK always sends `betas` as an HTTP header, never in the JSON body. Adding a body-based fallback would be dead code — Claude Code will never populate it.

#### Option B: Add diagnostic header logging + fix error translation (Recommended)

**Description:** Add comprehensive header logging to diagnose whether the `anthropic-beta` header is present in incoming requests. Simultaneously fix the `parse_prompt_too_long` secondary failure to prevent the 502 retry storm.

**Pros:**
- Identifies the true root cause of Failure 1 (header absent vs. present-but-missed)
- Immediately fixes Failure 2 (502 → 400 error translation)
- No speculative changes — acts on confirmed facts
- Minimal code changes, low risk

**Cons:**
- Does not fix Failure 1 immediately (requires a second pass after diagnostic data is collected)
- Requires reproducing the scenario to gather diagnostic data

#### Option C: Hardcode 1M model detection from model name (Not Recommended)

**Description:** If the incoming model name contains context-size indicators (e.g., version patterns that suggest 1M), auto-select the 1M Copilot model.

**Pros:**
- Works regardless of header issues

**Cons:**
- Fragile heuristic
- Model naming conventions may change
- Overrides user intent if they deliberately chose non-1M

### Recommended Approach

**Option B — diagnostic logging + error translation fix:**

1. Add diagnostic logging of incoming `anthropic-beta` headers (determines root cause of Failure 1)
2. Fix the `parse_prompt_too_long` bug (add more robust parsing + logging when parsing fails — fixes Failure 2)
3. After diagnostic data is collected, implement a targeted fix for Failure 1 based on findings
4. (Future) Add a `context_window` field to the `/v1/models` response so Claude Code can discover actual model limits

---

## Proposed Design / Architecture

### Component Overview

```
Claude Code                     copilot-adapter                    Copilot API
    │                                │                                 │
    │ POST /v1/messages              │                                 │
    │ model: claude-opus-4-6         │                                 │
    │ anthropic-beta: context-1m-*   │  ← FIX 1: log header value     │
    │                                │                                 │
    │                                │  model: claude-opus-4.6-1m      │
    │                                │ ──────────────────────────────→  │
    │                                │                                 │
    │                                │  400: prompt_too_long            │
    │                                │ ←──────────────────────────────  │
    │                                │  ← FIX 2: robust parsing        │
    │  400: prompt_too_long          │                                 │
    │ ←──────────────────────────────│                                 │
    │  (triggers compaction)         │                                 │
```

### Technical Details

#### Fix 1: Diagnostic logging for `anthropic-beta` header

In `src/handlers/messages.rs`, add logging after checking `has_1m_context_beta`:

```rust
let wants_1m = has_1m_context_beta(&headers);
tracing::debug!(
    wants_1m = wants_1m,
    anthropic_beta = ?headers.get_all("anthropic-beta")
        .iter()
        .map(|v| v.to_str().unwrap_or("<non-utf8>"))
        .collect::<Vec<_>>(),
    "Checked anthropic-beta header for 1M context"
);
```

#### Fix 2: Robust `parse_prompt_too_long` with failure logging

In `src/copilot/client.rs`, add logging when parsing fails:

```rust
if status.as_u16() == 400 {
    if let Some((actual, limit)) = parse_prompt_too_long(&body) {
        // ... existing success path ...
    } else if body.contains("model_max_prompt_tokens_exceeded") {
        // Body contains the expected error code but parsing failed.
        // Log diagnostics and fall back to regex-based extraction.
        tracing::warn!(
            body_len = body.len(),
            body_bytes = ?body.as_bytes().iter().take(20).collect::<Vec<_>>(),
            "parse_prompt_too_long failed despite matching error code, attempting regex fallback"
        );
        if let Some((actual, limit)) = parse_prompt_too_long_regex(&body) {
            return AppError::PromptTooLong {
                actual_tokens: actual,
                limit_tokens: limit,
            };
        }
    }
}
```

Add a regex-based fallback parser:

```rust
/// Regex-based fallback for prompt-too-long parsing.
/// Handles edge cases like invisible characters, alternative message formats,
/// or unexpected whitespace that the primary parser misses.
fn parse_prompt_too_long_regex(body: &str) -> Option<(u32, u32)> {
    let re = regex::Regex::new(
        r"prompt token count of (\d+) exceeds the limit of (\d+)"
    ).ok()?;
    let caps = re.captures(body)?;
    let actual: u32 = caps.get(1)?.as_str().parse().ok()?;
    let limit: u32 = caps.get(2)?.as_str().parse().ok()?;
    Some((actual, limit))
}
```

#### Fix 3: Context window in `/v1/models` response (future)

This is a larger change — enriching the `Model` struct with a `context_window` field so Claude Code can discover the actual model limits. Currently the `Model` struct only has `id`, `object`, `created`, `owned_by`. Adding `context_window` would let Claude Code set its autocompact thresholds correctly.

This requires either:
- The Copilot API's `/models` endpoint returning context window sizes (check if it does)
- A hardcoded mapping in the adapter from known model IDs to context windows

---

## Requirements

### Functional Requirements

| ID | Requirement | Source |
|----|-------------|--------|
| FR1 | Detect `context-1m-*` from `anthropic-beta` HTTP header (existing behavior — verify it works) | Log analysis |
| FR2 | Translate `model_max_prompt_tokens_exceeded` errors to Anthropic `prompt_too_long` format reliably | Log analysis |
| FR3 | Log `anthropic-beta` header value on every request for diagnostics | Debugging need |
| FR4 | Never return 502 for a 400-class upstream error | Error handling correctness |

### Non-Functional Requirements

| ID | Requirement | Target |
|----|-------------|--------|
| NFR1 | No performance regression from added logging | < 1ms overhead |
| NFR2 | Backward compatible — existing behavior preserved | Zero breaking changes |

---

## File Changes Summary

| File | Change | Description |
|------|--------|-------------|
| `src/handlers/messages.rs` | Modified | Add diagnostic `anthropic-beta` header logging |
| `src/copilot/client.rs` | Modified | Add regex fallback for `parse_prompt_too_long`; add failure logging |
| `src/handlers/models.rs` | Modified (future) | Add `context_window` to `Model` struct |

---

## Testing Strategy

### Unit Tests

1. **`parse_prompt_too_long` robustness:**
   - Body with trailing newline
   - Body with BOM prefix
   - Body with non-standard whitespace
   - Regex fallback for edge cases

### Integration Tests

1. **1M model selection:**
   - Request with `anthropic-beta: context-1m-2025-08-07` header → model becomes `*-1m`
   - Request without `anthropic-beta` header → model unchanged (no `-1m` suffix)

2. **Error translation:**
   - 400 with `model_max_prompt_tokens_exceeded` → HTTP 400 with `prompt_too_long`
   - Verify response format matches Claude Code's regex

### Manual E2E Tests

1. Start adapter with `--log-level debug`
2. Configure Claude Code with `/model opus[1m]`
3. Run a session until context approaches 168K tokens
4. Verify in logs: `"1M context beta detected"` appears
5. Verify model sent to Copilot API includes `-1m` suffix
6. If context limit hit, verify HTTP 400 returned (not 502)

---

## Risk Assessment

| Risk | Impact | Probability | Mitigation |
|------|--------|-------------|------------|
| Header absent due to Claude Code code path we can't fix | High | Medium | Diagnostic logging will confirm; if so, investigate Claude Code's API provider layer |
| Regex fallback has edge cases | Low | Low | Primary parser still used first; regex is secondary |
| Copilot API changes `model_max_prompt_tokens_exceeded` format | Medium | Low | Both string-based and regex parsers are resilient |
| `context_window` values hardcoded incorrectly | Medium | Medium | Prefer dynamic discovery from Copilot API |

---

## Success Criteria

1. **1M model activates** — When Claude Code sends the `anthropic-beta: context-1m-*` header, adapter logs show `"1M context beta detected"` and Copilot API request uses `*-1m` model
2. **Error translation works** — `model_max_prompt_tokens_exceeded` always returns HTTP 400 with `prompt_too_long` (never 502)
3. **Recovery possible** — Claude Code successfully compacts context and continues the conversation after a `prompt_too_long` error

---

## Design Decisions

| Decision | Rationale |
|----------|-----------|
| Do NOT add `betas` body fallback | Confirmed the SDK always sends `betas` as HTTP header; body fallback would be dead code |
| Add regex fallback for error parsing | The primary string parser fails in production for unknown reasons; regex is more resilient |
| Log all `anthropic-beta` header values | Essential for diagnosing whether header is absent (Claude Code issue) or present-but-missed (adapter issue) |
| Don't change how Claude Code selects models | Out of scope; the adapter should handle whatever Claude Code sends |

---

## Open Questions

| # | Question | Status |
|---|----------|--------|
| 1 | ~~Does the Anthropic SDK send `betas` as HTTP header or JSON body when using a custom `ANTHROPIC_BASE_URL`?~~ | **Resolved** — Always HTTP header, confirmed via SDK source and protocol spy logs |
| 2 | Why does `parse_prompt_too_long` fail on a body that matches the expected format? | Open — may need byte-level inspection of actual HTTP response |
| 3 | Does the Copilot API `/models` endpoint return `context_window` sizes? | Open — needs API investigation |
| 4 | Should the adapter enforce a maximum context window based on known model limits (pre-flight rejection)? | Deferred — would duplicate Claude Code's token counting |
| 5 | Which Claude Code code path omits the `anthropic-beta` header? | Open — diagnostic logging in the adapter will help identify this |

---

## References

- `docs/design/CONTEXT-WINDOW-AND-TRUNCATION.design.md` — Original design for prompt-too-long translation
- `src/copilot/client.rs` — `parse_prompt_too_long()` implementation
- `src/handlers/messages.rs` — `has_1m_context_beta()` implementation  
- `src/error.rs` — `AppError::PromptTooLong` error type
- Claude Code source: `src/utils/betas.ts` — Beta header construction
- Claude Code source: `src/utils/context.ts` — `has1mContext()` detection
- Claude Code source: `src/services/api/claude.ts` — `normalizeModelStringForAPI()` strips `[1m]`
- Claude Code source: `src/services/compact/autoCompact.ts` — Autocompact threshold computation
- Claude Code source: `src/services/api/errors.ts` — `parsePromptTooLongTokenCounts()` regex
