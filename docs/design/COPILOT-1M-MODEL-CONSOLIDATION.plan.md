# Copilot 1M Model Consolidation — Implementation Plan

**Status:** Done
**Date:** 2026-06-22
**Based on:** [COPILOT-1M-MODEL-CONSOLIDATION.design.md](./COPILOT-1M-MODEL-CONSOLIDATION.design.md)
**Estimated Time:** 0.5 day

---

## Executive Summary

Remove the now-broken Copilot model-name rewriting. GitHub Copilot consolidated
its Claude SKUs (no `-1m` / `-1m-internal` / `-high` / `-xhigh` model IDs), so
appending those suffixes selects non-existent models. This plan:

- Removes `apply_model_modifiers()` from `src/model_mapper.rs`.
- Removes both call sites in `src/handlers/messages.rs` (XML + native paths).
- Routes Opus 4.7 effort through the existing `reasoning.effort` translation.
- Updates unit + integration tests and documentation.

---

## Background

### Current State

- `src/model_mapper.rs::apply_model_modifiers(model, effort, wants_1m)` returns
  `(final_model_name, suppress_reasoning)`, appending `-1m` / `-1m-internal` or
  Opus 4.7 effort SKUs.
- Called in `src/handlers/messages.rs` XML path (~L182–206) and
  `handle_with_native_tools()` (~L1001–1023, with a `wants_1m: bool` param).

### Target State

- Function removed. Outgoing model name comes solely from
  `normalize_model_name()`. Effort flows through `reasoning.effort` for all
  models. `context-1m` header detected for diagnostics only.

---

## Implementation Plan

### Epic 1: Core code (Done)

#### Task 1.1: Remove `apply_model_modifiers()`

**File:** `src/model_mapper.rs` (MODIFIED)

- Delete the function + doc comment.
- Delete its inline `apply_model_modifiers tests` block.
- Keep `normalize_model_name()` and `is_datestamp()`.

**Acceptance Criteria:**
- [x] Function and tests removed
- [x] `normalize_model_name` untouched

#### Task 1.2: Remove both call sites

**File:** `src/handlers/messages.rs` (MODIFIED)

- XML path: delete the "Apply effort / 1M-context model modifiers" block.
- Native path: delete the same block; drop the `wants_1m: bool` parameter from
  `handle_with_native_tools()` and the argument at its call site.
- Keep `let wants_1m = has_1m_context_beta(&headers);` and the `anthropic-beta`
  diagnostic logging.

**Acceptance Criteria:**
- [x] No `apply_model_modifiers` references remain
- [x] `wants_1m` still used by diagnostic logging (no unused-var warning)

### Epic 2: Tests (Done)

#### Task 2.1: Unit tests

**File:** `tests/unit/messages_tests.rs` (MODIFIED)

- Replace append/guard tests with base-name + no-rewrite tests, plus an Opus 4.7
  base-name test.

#### Task 2.2: Integration tests

**File:** `tests/integration/messages_tests.rs` (MODIFIED)

- Update 3 expectations from `claude-opus-4.6-1m` → `claude-opus-4.6`.
- Repurpose the no-double-append test as an in-name `-1m` marker-preservation
  test (value unchanged; rationale updated).
- Add an Opus 4.7 + effort test asserting `model: claude-opus-4.7` +
  `reasoning.effort: xhigh`.

**Acceptance Criteria:**
- [x] `cargo test` passes

### Epic 3: Documentation (Done)

#### Task 3.1: Project docs

- `CLAUDE.md`: rewrite 1M note; update effort note.
- `docs/known-issues.md`: mark Opus 4.7 variants obsolete; update 1M section.
- `docs/development/e2e-testing.md`: rewrite Test 42.
- Supersede notes on `CONTEXT-WINDOW-AND-TRUNCATION.design.md` (Option C) and
  `CONTEXT-SIZE-MISMATCH.design.md` (Failure 1).
- `docs/design/backlog.md`: add Done entry.

**Acceptance Criteria:**
- [x] Docs reflect the no-rewrite behavior

---

## File Changes Summary

| File | Change | Epic |
|------|--------|------|
| `src/model_mapper.rs` | Modified | Epic 1 |
| `src/handlers/messages.rs` | Modified | Epic 1 |
| `tests/unit/messages_tests.rs` | Modified | Epic 2 |
| `tests/integration/messages_tests.rs` | Modified | Epic 2 |
| `CLAUDE.md` | Modified | Epic 3 |
| `docs/known-issues.md` | Modified | Epic 3 |
| `docs/development/e2e-testing.md` | Modified | Epic 3 |
| `docs/design/CONTEXT-WINDOW-AND-TRUNCATION.design.md` | Modified | Epic 3 |
| `docs/design/CONTEXT-SIZE-MISMATCH.design.md` | Modified | Epic 3 |
| `docs/design/backlog.md` | Modified | Epic 3 |
| `docs/design/COPILOT-1M-MODEL-CONSOLIDATION.design.md` | **New file** | Epic 3 |
| `docs/design/COPILOT-1M-MODEL-CONSOLIDATION.plan.md` | **New file** | Epic 3 |

---

## Success Criteria

1. Project compiles with `apply_model_modifiers` removed. (Epic 1)
2. `context-1m` header no longer changes the outgoing model name. (Epic 2)
3. Opus 4.7 effort routes via `reasoning.effort`. (Epic 2)
4. All unit + integration tests pass. (Epic 2)
5. Documentation updated. (Epic 3)

---

## References

- [Design document](./COPILOT-1M-MODEL-CONSOLIDATION.design.md)
- Anthropic models overview: https://platform.claude.com/docs/en/docs/about-claude/models/overview
- GitHub Copilot supported models: https://docs.github.com/en/copilot/reference/ai-models/supported-models
