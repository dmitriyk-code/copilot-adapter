# Copilot 1M Model Consolidation — Design Document

**Status:** Implemented
**Date:** 2026-06-22
**Severity:** High
**Related:** `docs/design/CONTEXT-WINDOW-AND-TRUNCATION.design.md` (Option C, superseded), `docs/design/CONTEXT-SIZE-MISMATCH.design.md` (Failure 1, superseded)

---

## Executive Summary

GitHub Copilot consolidated its Claude model SKUs. The live
`GET https://api.githubcopilot.com/models` response now exposes **only base
model names** (`claude-opus-4.5/4.6/4.7/4.8`, `claude-sonnet-4.5/4.6`,
`claude-haiku-4.5`) — there are **no** `-1m`, `-1m-internal`, `-high`, or
`-xhigh` model IDs anymore.

The adapter previously rewrote the outgoing Copilot model name in
`apply_model_modifiers()`:
- Appended `-1m` (or `-1m-internal` for Opus 4.7) when the
  `anthropic-beta: context-1m-*` header was present.
- Encoded Opus 4.7 effort as model-name SKUs (`claude-opus-4.7-high` / `-xhigh`).

Both behaviors now produce **model-not-found** names that Copilot rejects. A user
selecting "Opus (1M context)" or setting `/effort` on Opus 4.7 through the
adapter got a broken request.

**Fix:** remove `apply_model_modifiers()` entirely. The adapter still *detects*
the `context-1m` header for diagnostic logging, but forwards the normalized base
model name unchanged. Effort for all models (including Opus 4.7) flows through
the standard `reasoning.effort` field, which `to_chat_completion_request()`
already populates.

---

## Context / Background

### Current State (before this change)

`src/model_mapper.rs::apply_model_modifiers(model, effort, wants_1m)` returned a
`(final_model_name, suppress_reasoning)` tuple:
- Opus 4.7 + 1M header → `claude-opus-4.7-1m-internal` (suppress reasoning)
- Opus 4.7 + effort `xhigh`/`high`/`max` → `claude-opus-4.7-xhigh` / `-high`
- Any other model + 1M header → `{model}-1m` (e.g., `claude-opus-4.6-1m`)

Called from two sites in `src/handlers/messages.rs` (XML path and native-tools
path). When it changed the model name, it also cleared `openai_request.reasoning`
for the effort-suffix cases.

### Target State

- `apply_model_modifiers()` is removed.
- The messages handler still computes `wants_1m = has_1m_context_beta(&headers)`
  and logs the `anthropic-beta` header values for diagnostics, but does not use
  the result to alter the model name.
- The outgoing model name is whatever `normalize_model_name()` produces.
- Effort is translated to `reasoning.effort` for all models (existing logic in
  `to_chat_completion_request()`; `"max"` → `"high"`, others including `"xhigh"`
  pass through).

---

## Problem Statement

**Observed behavior:** Selecting a 1M-context Claude model in Claude Code, or
setting an effort level on Opus 4.7, causes the adapter to send a model name like
`claude-opus-4.6-1m` / `claude-opus-4.7-1m-internal` / `claude-opus-4.7-xhigh` —
none of which exist in Copilot's current model catalogue. Copilot returns a
model-not-found error.

**Expected behavior:** The adapter sends the base model name (e.g.,
`claude-opus-4.6`, `claude-opus-4.7`), which is 1M-native, and passes effort via
`reasoning.effort`.

**Impact:** Any user selecting 1M context or using Opus 4.7 with a non-default
effort through the adapter gets failing requests.

---

## Goals and Non-Goals

### Goals

| ID | Goal | Success Criteria |
|----|------|------------------|
| G1 | Stop appending `-1m` / `-1m-internal` for the 1M header | Outgoing model name equals the normalized base name when the `context-1m` header is present |
| G2 | Stop encoding Opus 4.7 effort as model-name SKUs | Opus 4.7 + effort sends `model: claude-opus-4.7` + `reasoning.effort` |
| G3 | Preserve `context-1m` diagnostic logging | `wants_1m` + `anthropic-beta` values still logged |
| G4 | No regressions in effort/thinking translation | Existing effort/thinking tests pass |

### Non-Goals

| ID | Non-Goal | Rationale |
|----|----------|-----------|
| NG1 | Changing `normalize_model_name()` in-name `-1m` marker preservation | Unreachable from Claude Code; out of scope |
| NG2 | Adding per-model context-window data to `/v1/models` | Separate deferred item (see CONTEXT-SIZE-MISMATCH) |
| NG3 | Touching prompt-too-long / truncated-tool logic | Unrelated |

---

## Research / Analysis

### Key Findings

1. **Copilot `/models` has no suffixed Claude IDs.** A live response lists only
   base names: `claude-opus-4.5`, `claude-opus-4.6`, `claude-opus-4.7`,
   `claude-opus-4.8`, `claude-sonnet-4.5`, `claude-sonnet-4.6`,
   `claude-haiku-4.5`. No `-1m`, `-1m-internal`, `-high`, or `-xhigh`.

2. **Anthropic docs — base models are 1M-native.**
   [Models overview](https://platform.claude.com/docs/en/docs/about-claude/models/overview):
   Opus 4.6, 4.7, 4.8 and Sonnet 4.6 have a **1M-token** context window; Opus
   4.5, Sonnet 4.5, and Haiku 4.5 are 200k. There is no separate "1M model" — the
   capability is intrinsic to the model.

3. **GitHub Copilot docs — 1M is a context-size toggle.**
   [Supported models](https://docs.github.com/en/copilot/reference/ai-models/supported-models):
   1M context is offered for Sonnet 4.6 and Opus 4.6/4.7/4.8 as a choice between
   "default" and "extended (1M token)" context **after** selecting the base
   model — not as a distinct model ID.

4. **Effort already translates to `reasoning.effort`.**
   `src/anthropic/types.rs::to_chat_completion_request()` maps
   `output_config.effort` → `reasoning.effort` for every model (`"max"`→`"high"`,
   others pass through). Removing the Opus 4.7 special case in
   `apply_model_modifiers()` makes 4.7 effort flow through this existing path
   with no new code.

### Options Considered

#### Option A: Remove `apply_model_modifiers()` entirely (Recommended)

**Description:** Delete the function and both call sites. Rely on
`normalize_model_name()` for the model name and the existing
`reasoning.effort` translation for effort.

**Pros:**
- Simplest; removes dead/harmful code.
- Effort path already exists and is well-tested.
- Matches Copilot's consolidated catalogue.

**Cons:**
- Reverses a previously documented design (Option C of
  CONTEXT-WINDOW-AND-TRUNCATION). Mitigated by superseding notes.

#### Option B: Keep `apply_model_modifiers()` but make it a no-op for 1M only

**Description:** Drop the `-1m` append but keep the Opus 4.7 effort SKUs.

**Pros:** Smaller diff if the 4.7 SKUs still existed.

**Cons:** The 4.7 effort SKUs are also gone from `/models`, so this would leave a
known-broken path. Rejected.

### Recommended Approach

Option A — remove the function. Scope confirmed with the user: apply to **all**
Claude models and **also** drop the Opus 4.7 effort suffixes.

---

## Proposed Design / Architecture

### Component Overview

```
Claude Code                      copilot-adapter                     Copilot API
   │  POST /v1/messages              │                                   │
   │  model: claude-opus-4-6         │                                   │
   │  anthropic-beta: context-1m-*   │  has_1m_context_beta() → log only │
   │  output_config.effort: high     │                                   │
   │                                 │  normalize_model_name             │
   │                                 │    → claude-opus-4.6              │
   │                                 │  reasoning.effort: high           │
   │                                 │ ─────────────────────────────────→│
```

### Technical Details

- **`src/model_mapper.rs`**: delete `apply_model_modifiers()` and its inline
  tests. Keep `normalize_model_name()` (including in-name `-1m` preservation).
- **`src/handlers/messages.rs`**: remove the modifier block from both the XML
  path and `handle_with_native_tools()`; drop the `wants_1m` parameter from
  `handle_with_native_tools()`. Keep `wants_1m` computation + diagnostic logging
  in the main handler.

---

## File Changes Summary

| File | Change | Description |
|------|--------|-------------|
| `src/model_mapper.rs` | Modified | Remove `apply_model_modifiers()` + its inline tests |
| `src/handlers/messages.rs` | Modified | Remove both modifier call sites; drop `wants_1m` param from `handle_with_native_tools` |
| `tests/unit/messages_tests.rs` | Modified | Replace append/guard tests with base-name + no-rewrite tests |
| `tests/integration/messages_tests.rs` | Modified | Update 3 expectations to base name; rename/repurpose the marker-preservation test; add Opus 4.7 effort test |
| `CLAUDE.md` | Modified | Rewrite 1M note; update effort note |
| `docs/known-issues.md` | Modified | Mark Opus 4.7 variants obsolete; update 1M section |
| `docs/development/e2e-testing.md` | Modified | Rewrite Test 42 |
| `docs/design/CONTEXT-WINDOW-AND-TRUNCATION.design.md` | Modified | Supersede note on Option C |
| `docs/design/CONTEXT-SIZE-MISMATCH.design.md` | Modified | Supersede note on Failure 1 |
| `docs/design/backlog.md` | Modified | Add Done entry |

---

## Testing Strategy

### Unit Tests

- `tests/unit/messages_tests.rs`: `normalize_model_name` yields base names;
  the `context-1m` header does not change the model name; Opus 4.7 normalizes to
  `claude-opus-4.7`.

### Integration Tests

- `tests/integration/messages_tests.rs`: with the `context-1m` header, the mock
  Copilot receives `claude-opus-4.6` (non-streaming, streaming, captured-request).
- In-name `-1m` marker still preserved (`claude-opus-4-6-1m-...` → `claude-opus-4.6-1m`).
- Opus 4.7 + effort `xhigh` → outgoing `model: claude-opus-4.7` and
  `reasoning.effort: xhigh`.

### Manual E2E

- e2e Test 42 (rewritten): diagnostic header log fires; outgoing model is the
  base name; Copilot accepts it.

---

## Risk Assessment

| Risk | Impact | Probability | Mitigation |
|------|--------|-------------|------------|
| A future Copilot catalogue re-introduces suffixed SKUs | Medium | Low | Re-add targeted routing if/when `/models` shows them |
| Hidden caller depended on the removed function | Low | Very Low | Only two in-repo call sites; both updated; compiler enforces |
| Effort `xhigh` rejected by Copilot for some model | Low | Low | Same passthrough behavior as before for non-4.7 models; no new values introduced |

---

## Success Criteria

1. `apply_model_modifiers()` no longer exists; project compiles.
2. With `context-1m`, the outgoing model name is the normalized base name.
3. Opus 4.7 + effort sends `model: claude-opus-4.7` + `reasoning.effort`.
4. All unit + integration tests pass.

---

## Design Decisions

| Decision | Rationale |
|----------|-----------|
| Remove the function rather than no-op it | All its branches now target non-existent models; deletion is clearest |
| Apply to all Claude models, not just Opus | No `-1m` IDs exist for any model; appending always breaks |
| Also drop Opus 4.7 effort SKUs | Those SKUs are gone too; `reasoning.effort` is the correct, uniform path |
| Keep `normalize_model_name` in-name `-1m` preservation | Unreachable from Claude Code; harmless; out of scope |
| Keep `context-1m` diagnostic logging | Still useful for debugging 1M-selection issues |

---

## Open Questions

| # | Question | Status |
|---|----------|--------|
| 1 | Does Copilot enforce a smaller prompt limit than the model's native 1M? | Open — handled separately by `prompt_too_long` translation regardless |
| 2 | Will Copilot re-introduce effort/context SKUs later? | Open — revisit if `/models` changes |

---

## References

- Anthropic models overview: https://platform.claude.com/docs/en/docs/about-claude/models/overview
- GitHub Copilot supported models: https://docs.github.com/en/copilot/reference/ai-models/supported-models
- `src/model_mapper.rs` — `normalize_model_name()` (retained)
- `src/handlers/messages.rs` — `has_1m_context_beta()` (retained), modifier call sites (removed)
- `src/anthropic/types.rs` — `to_chat_completion_request()` effort → `reasoning.effort`
- Superseded: `CONTEXT-WINDOW-AND-TRUNCATION.design.md` (Option C), `CONTEXT-SIZE-MISMATCH.design.md` (Failure 1)
