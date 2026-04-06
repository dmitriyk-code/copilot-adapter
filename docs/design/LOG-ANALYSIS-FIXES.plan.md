# Log Analysis Fixes — Implementation Plan

**Status:** In Progress
**Date:** 2026-04-06
**Based on:** [Log analysis findings](../../log-analysis-2026-04-06.md) — Issues 2 and 3
**Prerequisite:** None
**Estimated Time:** 1-2 days

---

## Executive Summary

Log analysis of 12 log files (~1,200 requests over 4 days) identified two open issues attributable to the copilot-adapter. This plan fixes both:

- **Issue 2 — Token refresh race condition:** The proactive `start_auto_refresh()` background task already exists in `src/auth/token.rs` but is never started. Without it, tokens are only refreshed lazily when a request arrives — and if no request arrives before expiry, the next request receives a 401 → 502. The fix is simply to start the background task at server startup.

- **Issue 3 — System prompt concatenation without separators:** In `src/anthropic/types.rs`, the `SystemInput::to_text()` method joins multiple system text blocks with `.join("")` — no separator of any kind. This causes adjacent blocks to run together (e.g., `...cch=00000;You are Claude Code...`) on every request that uses a multi-block system prompt. The fix is to use `"\n\n"` as the separator.

This plan implements:
- Start the background auto-refresh task at server startup
- Add `"\n\n"` separator between system prompt blocks in `SystemInput::to_text()`
- Add `"\n\n"` separator in `extract_text()` for consistency
- Tests for both fixes

**Total estimated time:** 1-2 days

---

## Background

### Current State

**Token refresh (Issue 2):**
- `TokenManager` in `src/auth/token.rs` has a `start_auto_refresh()` method (lines 140–202) that spawns a background `tokio` task
- The task calculates `secs_until_expiry - 300` and sleeps until 5 minutes before the token expires
- However, this method is **never called** — `AppState` is constructed in `src/server.rs` without starting the background task
- Tokens are therefore only refreshed lazily inside `get_valid_token()` when a request is processed
- Evidence: a 401 error was observed in `logs5.txt` when a ~30-minute gap in traffic allowed the token to expire before the next request arrived

**System prompt concatenation (Issue 3):**
- `SystemInput::to_text()` in `src/anthropic/types.rs` (lines 76–91) joins multiple system text blocks using `.join("")`
- No separator of any kind is inserted between blocks
- Result: adjacent blocks run together — e.g., a billing header block ending with `cch=00000;` immediately followed by `You are Claude Code` (no whitespace), and instructions like `...CLI for Claude.Generate a concise title...`
- This affects every request that uses a multi-block `system` field, which is the common Claude Code pattern
- The same issue exists in `extract_text()` (lines 468–485), which handles user/assistant message content

### Target State

**Token refresh (Issue 2):**
- The background auto-refresh task is started at server startup alongside the HTTP server
- Tokens are proactively refreshed 5 minutes before expiry, independent of incoming traffic
- No 401 errors caused by expiry during idle periods
- The task is cancelled gracefully on server shutdown

**System prompt concatenation (Issue 3):**
- `SystemInput::to_text()` uses `"\n\n"` as the separator between text blocks
- `extract_text()` uses `"\n\n"` for consistency
- System prompt sections are cleanly delimited, improving model interpretation

---

## Goals and Non-Goals

### Goals

| ID | Goal | Success Criteria |
|----|------|------------------|
| G1 | Proactive token refresh eliminates expiry-related 401 errors | No 401 from Copilot API during idle periods; background task visible in startup logs |
| G2 | Background task shuts down cleanly | No lingering tasks after server shutdown signal; no log noise |
| G3 | System prompt blocks are separated by `"\n\n"` | Outgoing OpenAI `system` content shows clear block boundaries in trace logs |
| G4 | Fix is consistent across all `join("")` call sites in the translation layer | `SystemInput::to_text()` and `extract_text()` both use the separator |

### Non-Goals

| ID | Non-Goal | Rationale |
|----|----------|-----------|
| NG1 | Redesign the token refresh architecture | `start_auto_refresh()` already exists and is correct; just needs to be called |
| NG2 | Add separator to `extract_tool_result_messages()` inner join | Tool result content blocks are typically single-element; fixing system prompt is higher priority |
| NG3 | Make the separator configurable | `"\n\n"` is the standard Markdown block separator; no need for configuration |
| NG4 | Fix tool_result inner block join (line 603) | Separate, lower-priority concern; tool result blocks rarely have multiple text blocks |

---

## Implementation Plan

### Epic 1: Token Refresh Background Task (Day 1, 0.5 days)

**Status:** Complete

**Objective:** Start the `start_auto_refresh()` background task during server startup so tokens are refreshed proactively.

#### Task 1.1: Start Auto-Refresh Task in Server Startup

**File:** `src/server.rs` (MODIFIED)

**Description:** After constructing `AppState`, call `token_manager.clone().start_auto_refresh()` to spawn the background refresh task. Store the returned `JoinHandle` alongside the server's shutdown logic so it is cancelled when the server stops.

The `TokenManager` already has full infrastructure for this:
- `start_auto_refresh()` takes `Arc<Self>` and returns a `JoinHandle<()>`
- It uses a `CancellationToken` stored in `self.cancel` that is cloned into the background task
- The existing `stop_auto_refresh()` method on `TokenManager` cancels this token

**Implementation:**
```rust
// In run_server() or equivalent, after AppState construction:

// Before (no auto-refresh):
let state = Arc::new(AppState {
    token_manager,
    copilot_client: CopilotClient::new(http_client),
    config,
    models_cache,
    conversation_logger,
});

// After (start auto-refresh):
let state = Arc::new(AppState {
    token_manager,
    copilot_client: CopilotClient::new(http_client),
    config,
    models_cache,
    conversation_logger,
});

// Start background token refresh task
let _refresh_handle = state.token_manager.clone().start_auto_refresh();
tracing::info!("Token auto-refresh task started");
```

**Shutdown integration:** `stop_auto_refresh()` should be called on graceful shutdown. The shutdown signal handler in `src/server.rs` already waits for `ctrl_c` / SIGTERM — call `state.token_manager.stop_auto_refresh()` there.

**Implementation (shutdown):**
```rust
// In the shutdown handler (after signal received):

// Before:
tracing::info!("Shutdown signal received, stopping server");

// After:
tracing::info!("Shutdown signal received, stopping server");
state.token_manager.stop_auto_refresh();
tracing::debug!("Token auto-refresh task cancelled");
```

**Acceptance Criteria:**
- [x] Server startup log includes `"Token auto-refresh task started"`
- [x] First token refresh is proactive (fires before expiry, not triggered by a request)
- [x] Shutdown log includes cancellation of the refresh task (no task leak)
- [x] No race condition between background refresh and request-driven `get_valid_token()`
- [x] Unit tests passing

**Notes:** The `refresh_lock` Mutex in `TokenManager` already serializes background and request-driven refresh calls — no additional synchronization needed.

---

#### Task 1.2: Verify `seconds_until_expiry()` Behaviour

**File:** `src/auth/token.rs` (review only, likely no change)

**Description:** Confirm that `CopilotToken::seconds_until_expiry()` (called by `start_auto_refresh()`) returns a `u64` correctly and handles the edge case where the token is already expired (returns 0 rather than underflowing). Verify the task handles a missing initial token gracefully (no token yet — the task should wait 60 seconds and retry).

**Acceptance Criteria:**
- [x] `seconds_until_expiry()` returns `0` (not underflow) when token is already expired
- [x] Background task logs at DEBUG level when no token is present and it waits 60s
- [x] No panic on first startup before a token is acquired

---

### Epic 2: System Prompt Separator Fix (Day 1, 0.5 days)

**Status:** Done

**Objective:** Add `"\n\n"` as the separator when joining multiple system text blocks in `SystemInput::to_text()` and the `extract_text()` helper.

#### Task 2.1: Fix `SystemInput::to_text()`

**File:** `src/anthropic/types.rs` (MODIFIED)

**Description:** Change the `.join("")` call in `SystemInput::to_text()` to `.join("\n\n")`. This is the primary fix that affects the OpenAI `system` message content.

**Implementation:**
```rust
// Before (lines 76-91):
impl SystemInput {
    pub fn to_text(&self) -> String {
        match self {
            SystemInput::Text(s) => s.clone(),
            SystemInput::Blocks(blocks) => blocks
                .iter()
                .filter_map(|b| match b {
                    ContentBlock::Text { text, .. } => Some(text.as_str()),
                    _ => None,
                })
                .collect::<Vec<_>>()
                .join(""),   // ← no separator
        }
    }
}

// After:
impl SystemInput {
    pub fn to_text(&self) -> String {
        match self {
            SystemInput::Text(s) => s.clone(),
            SystemInput::Blocks(blocks) => blocks
                .iter()
                .filter_map(|b| match b {
                    ContentBlock::Text { text, .. } => Some(text.as_str()),
                    _ => None,
                })
                .collect::<Vec<_>>()
                .join("\n\n"),  // ← double newline separator between blocks
        }
    }
}
```

**Acceptance Criteria:**
- [x] `SystemInput::Blocks` with 2+ text blocks are joined with `"\n\n"`
- [x] `SystemInput::Text` (single string) is unchanged
- [x] Single-block arrays produce no trailing separator
- [x] Trace logs show clean block boundaries in the outgoing OpenAI `system` content
- [x] Unit tests passing

**Notes:** Claude Code's system prompt routinely has 3–5 text blocks: billing header, identity, instructions, session-specific guidance, and tool descriptions. `"\n\n"` is the standard Markdown paragraph separator and is the natural choice for delimiting independent instruction blocks.

---

#### Task 2.2: Fix `extract_text()` Helper

**File:** `src/anthropic/types.rs` (MODIFIED)

**Description:** Change the `.join("")` call in the private `extract_text()` function (lines 468–485) to `.join("\n\n")`. This function handles user/assistant message content blocks — the same concatenation problem can arise there.

**Implementation:**
```rust
// Before (lines 468-485):
fn extract_text(content: &ContentBlockInput) -> String {
    match content {
        ContentBlockInput::Text(s) => s.clone(),
        ContentBlockInput::Blocks(blocks) => blocks
            .iter()
            .filter_map(|b| match b {
                ContentBlock::Text { text, .. } => Some(text.clone()),
                ContentBlock::Image { .. } => Some("[Image]".to_string()),
                ContentBlock::Document { title, .. } => {
                    Some(title.clone().unwrap_or_else(|| "[Document]".to_string()))
                }
                _ => None,
            })
            .collect::<Vec<_>>()
            .join(""),   // ← no separator
    }
}

// After:
fn extract_text(content: &ContentBlockInput) -> String {
    match content {
        ContentBlockInput::Text(s) => s.clone(),
        ContentBlockInput::Blocks(blocks) => blocks
            .iter()
            .filter_map(|b| match b {
                ContentBlock::Text { text, .. } => Some(text.clone()),
                ContentBlock::Image { .. } => Some("[Image]".to_string()),
                ContentBlock::Document { title, .. } => {
                    Some(title.clone().unwrap_or_else(|| "[Document]".to_string()))
                }
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("\n\n"),  // ← double newline separator between blocks
    }
}
```

**Acceptance Criteria:**
- [x] Multiple text blocks in message content are joined with `"\n\n"`
- [x] Image and Document placeholder strings are separated from adjacent text blocks
- [x] Unit tests passing

---

### Epic 3: Testing (Day 2, 0.5 days)

**Status:** Not Started

**Objective:** Ensure both fixes are tested correctly.

#### Task 3.1: Unit Tests

**File:** `tests/unit/` (MODIFIED — add to existing test files)

**Tests to implement:**

1. **Auto-refresh task starts and produces proactive refreshes:**
   ```rust
   #[tokio::test]
   async fn auto_refresh_task_refreshes_before_expiry() {
       // Set up a TokenManager with a mock token expiring in 10 seconds
       // Start auto-refresh with a mock auth client
       // Wait 6 seconds (within the "5 min before" window)
       // Verify the mock auth client was called (refresh triggered)
   }
   ```
   - [ ] Test passes

2. **Auto-refresh task cancels cleanly on shutdown:**
   ```rust
   #[tokio::test]
   async fn auto_refresh_task_cancels_on_shutdown() {
       // Start auto-refresh task
       // Call stop_auto_refresh()
       // Verify the JoinHandle completes within a short timeout
   }
   ```
   - [ ] Test passes

3. **`SystemInput::to_text()` — single block (no extra separators):**
   ```rust
   #[test]
   fn system_input_single_block_no_trailing_separator() {
       let input = SystemInput::Blocks(vec![ContentBlock::Text {
           text: "Hello world".to_string(),
           cache_control: None,
       }]);
       assert_eq!(input.to_text(), "Hello world");
   }
   ```
   - [ ] Test passes

4. **`SystemInput::to_text()` — multiple blocks joined with `"\n\n"`:**
   ```rust
   #[test]
   fn system_input_multiple_blocks_joined_with_double_newline() {
       let input = SystemInput::Blocks(vec![
           ContentBlock::Text { text: "Block one.".to_string(), cache_control: None },
           ContentBlock::Text { text: "Block two.".to_string(), cache_control: None },
           ContentBlock::Text { text: "Block three.".to_string(), cache_control: None },
       ]);
       assert_eq!(input.to_text(), "Block one.\n\nBlock two.\n\nBlock three.");
   }
   ```
   - [ ] Test passes

5. **`SystemInput::to_text()` — non-text blocks filtered out:**
   ```rust
   #[test]
   fn system_input_filters_non_text_blocks() {
       let input = SystemInput::Blocks(vec![
           ContentBlock::Text { text: "Text block.".to_string(), cache_control: None },
           ContentBlock::Image { /* ... */ },
           ContentBlock::Text { text: "Another block.".to_string(), cache_control: None },
       ]);
       // Image block is skipped; adjacent text blocks still get separator
       assert_eq!(input.to_text(), "Text block.\n\nAnother block.");
   }
   ```
   - [ ] Test passes

6. **`SystemInput::Text` passthrough (unchanged):**
   ```rust
   #[test]
   fn system_input_text_variant_is_unchanged() {
       let input = SystemInput::Text("Plain string system prompt".to_string());
       assert_eq!(input.to_text(), "Plain string system prompt");
   }
   ```
   - [ ] Test passes

**Acceptance Criteria:**
- [ ] All unit tests passing
- [ ] No regressions in existing `anthropic/types.rs` tests

#### Task 3.2: Integration Tests

No new integration test files needed — the existing request/response translation tests should cover the separator change.

**Verification via existing trace logging:**
- Start adapter with `--log-level trace`
- Send a request with a multi-block system prompt
- Verify the outgoing OpenAI `system` message content in the trace log shows `"\n\n"` between blocks

**Acceptance Criteria:**
- [ ] Existing integration tests still pass
- [ ] Trace log shows correct separators in `system` content

#### Task 3.3: Manual E2E Tests

**File:** `docs/e2e-testing.md` (MODIFIED)

**Test procedures to add:**

1. **Verify proactive token refresh:**
   ```bash
   # Start adapter with debug logging
   copilot-adapter start --log-level debug
   # Look for: "Token auto-refresh task started" in startup logs
   # Wait ~25 minutes (let the token approach expiry)
   # Look for: "Copilot token auto-refreshed successfully" in logs
   # WITHOUT making any requests during this period
   ```
   - Expected: Refresh log appears ~5 minutes before the token's 30-min expiry, without requiring a request

2. **Verify system prompt block separation:**
   ```bash
   # Start adapter with trace logging
   copilot-adapter start --log-level trace
   # Make any request through Claude Code
   # In the trace log, find "OUTGOING" direction for the system message
   # Expected: blocks separated by \n\n, not concatenated
   ```
   - Expected: `"content": "...cch=00000;\n\nYou are Claude Code..."` (with separator)

**Acceptance Criteria:**
- [ ] E2E test procedures documented
- [ ] Manual tests executed and verified

---

### Epic 4: Documentation (Day 2, 0.5 days)

**Status:** Not Started

**Objective:** Update documentation to reflect both fixes.

#### Task 4.1: Update CLAUDE.md

**File:** `CLAUDE.md` (MODIFIED)

**Changes:**
- Under **Notes for Development**, update the token management note to mention that `start_auto_refresh()` is called at server startup
- Remove any language suggesting proactive refresh is "not yet enabled"

**Acceptance Criteria:**
- [ ] CLAUDE.md accurately describes the token refresh behavior (background task + request-driven fallback)

---

## Requirements

### Functional Requirements

| ID | Requirement | Source | Epic |
|----|-------------|--------|------|
| FR1 | Background token refresh task starts at server startup | Log analysis Issue 2 | Epic 1 |
| FR2 | Background task refreshes the token 5 minutes before expiry | `start_auto_refresh()` existing logic | Epic 1 |
| FR3 | Background task is cancelled on graceful server shutdown | Log analysis Issue 2 | Epic 1 |
| FR4 | System text blocks are separated by `"\n\n"` in the OpenAI system message | Log analysis Issue 3 | Epic 2 |
| FR5 | User/assistant message content blocks are separated by `"\n\n"` | Log analysis Issue 3 | Epic 2 |

### Non-Functional Requirements

| ID | Requirement | Target | Epic |
|----|-------------|--------|------|
| NFR1 | No 401 errors during idle periods | Zero 401 from Copilot API when background task is running | Epic 1 |
| NFR2 | Background task does not increase request latency | Token checked in-memory, no blocking | Epic 1 |
| NFR3 | Separator change does not break existing requests | Existing tests pass; models handle `"\n\n"` separator | Epic 2 |

---

## File Changes Summary

| File | Change | Epic | Description |
|------|--------|------|-------------|
| `src/server.rs` | Modified | Epic 1 | Start `start_auto_refresh()` at server startup; call `stop_auto_refresh()` on shutdown |
| `src/anthropic/types.rs` | Modified | Epic 2 | Change `.join("")` to `.join("\n\n")` in `SystemInput::to_text()`, `extract_text()`, and `extract_tool_result_messages()` |
| `tests/unit/` | Modified | Epic 3 | Add unit tests for separator fix and auto-refresh task |
| `docs/e2e-testing.md` | Modified | Epic 3 | Add E2E test procedures |
| `CLAUDE.md` | Modified | Epic 4 | Update token refresh documentation |

---

## Testing Strategy

### Test Coverage

| Component | Unit Tests | Integration Tests | E2E Tests |
|-----------|------------|-------------------|-----------|
| Auto-refresh task lifecycle | Epic 3.1 (#1, #2) | — | Epic 3.3 (#1) |
| `SystemInput::to_text()` | Epic 3.1 (#3–#6) | Epic 3.2 | Epic 3.3 (#2) |
| `extract_text()` | Epic 3.1 (#3–#5 variant) | Epic 3.2 | — |

### Test Files

| File | Type | Coverage |
|------|------|----------|
| `tests/unit/` | Unit | Separator variants, auto-refresh lifecycle |
| `tests/integration/` | Integration | Full request translation with system blocks |
| `docs/e2e-testing.md` | Manual E2E | Idle-period refresh, trace log separator check |

---

## Dependencies

### External Dependencies

None — both fixes use only existing crates and internal modules.

### Internal Dependencies

| Module | Required By | Status |
|--------|-------------|--------|
| `src/auth/token.rs` — `TokenManager::start_auto_refresh()` | Epic 1 | ✅ Exists (just not called) |
| `src/auth/token.rs` — `TokenManager::stop_auto_refresh()` | Epic 1 | ✅ Exists |
| `src/anthropic/types.rs` — `SystemInput::to_text()` | Epic 2 | ✅ Exists (1-line change) |
| `src/anthropic/types.rs` — `extract_text()` | Epic 2 | ✅ Exists (1-line change) |

---

## Risk Assessment

| Risk | Impact | Probability | Mitigation | Epic |
|------|--------|-------------|------------|------|
| Background refresh task panics and silently exits | High | Low | `start_auto_refresh()` wraps refresh in `match`; errors logged, task continues looping | Epic 1 |
| Race between background refresh and concurrent request refresh | Low | Low | Already handled by `refresh_lock` Mutex inside `TokenManager` | Epic 1 |
| `"\n\n"` separator breaks a model that was relying on the run-on format | Very Low | Very Low | Run-on prompts are a degraded form; `"\n\n"` is strictly better structured input | Epic 2 |
| Separator causes token count to increase measurably | Very Low | Low | Each added `"\n\n"` is 2 tokens; with ~3 system blocks, increase is ~4 tokens (<0.01% of context) | Epic 2 |

---

## Success Criteria

1. **No idle-period 401 errors** — Background refresh task fires before token expires even without incoming requests (Epic 1)
2. **Clean startup and shutdown logs** — `"Token auto-refresh task started"` on startup, no leaked tasks on shutdown (Epic 1)
3. **System prompt blocks are delimited** — Outgoing OpenAI trace logs show `"\n\n"` between system text blocks (Epic 2)
4. **All tests passing** — Unit tests for separator and auto-refresh lifecycle, integration tests unchanged (Epic 3)
5. **Documentation updated** — CLAUDE.md reflects current auto-refresh behavior (Epic 4)

---

## Rollout / Migration Plan

### Phase 1: Development (Epics 1-2)
- [x] Start auto-refresh in `src/server.rs`
- [x] Change `.join("")` → `.join("\n\n")` in three locations in `src/anthropic/types.rs`
- [ ] Code review

### Phase 2: Testing (Epic 3)
- [ ] Unit tests complete
- [ ] Integration tests pass (no regressions)
- [ ] Manual E2E: confirm proactive refresh fires during idle period
- [ ] Manual E2E: confirm trace logs show `"\n\n"` separators

### Phase 3: Documentation (Epic 4)
- [ ] CLAUDE.md updated

### Phase 4: Release
- [ ] All acceptance criteria met
- [ ] Merge to main

---

## Epic Status Tracking

| Epic | Status | Start Date | End Date | Notes |
|------|--------|------------|----------|-------|
| Epic 1: Token Refresh Background Task | Done | 2026-04-06 | 2026-04-06 | stop_auto_refresh() called after axum::serve() returns (better than inside shutdown_signal()) |
| Epic 2: System Prompt Separator Fix | Done | 2026-04-06 | 2026-04-06 | Changed .join("") to .join("\n\n") in three locations: SystemInput::to_text(), extract_text(), and extract_tool_result_messages() |
| Epic 3: Testing | Not Started | - | - | |
| Epic 4: Documentation | Not Started | - | - | |

---

## Open Questions

| # | Question | Status | Blocker For |
|---|----------|--------|-------------|
| 1 | Does `CopilotToken::seconds_until_expiry()` handle the underflow case (already expired)? | Open | Epic 1 |
| 2 | Should `extract_tool_result_messages()` inner join (line 603) also be fixed with `"\n\n"`? | Resolved — implemented in Epic 2 | — |

---

## References

- [Log analysis report](../../log-analysis-2026-04-06.md) — Source of both issues
- `src/auth/token.rs` — `TokenManager`, `start_auto_refresh()`, `stop_auto_refresh()`
- `src/anthropic/types.rs` — `SystemInput::to_text()`, `extract_text()`
- `src/server.rs` — `AppState` construction and server lifecycle

---

## Notes

### Development Notes
- Both changes are small in scope: Epic 1 is ~5 lines in `src/server.rs`; Epic 2 is 2 one-line changes in `src/anthropic/types.rs`
- The token auto-refresh infrastructure was clearly designed to be enabled — `start_auto_refresh()` and `stop_auto_refresh()` are both implemented and well-commented; this is a missing integration step, not a design gap
- The `"\n\n"` separator choice matches the Markdown convention for paragraph separation and is consistent with how Claude Code itself structures multi-section content
