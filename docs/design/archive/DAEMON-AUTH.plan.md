# Daemon Authentication Fix — Implementation Plan

**Status:** Not Started
**Date:** 2026-04-01
**Based on:** [DAEMON-AUTH.design.md](./DAEMON-AUTH.design.md)
**Prerequisite:** None
**Estimated Time:** 0.5 days

---

## Executive Summary

This plan removes the daemon-specific authentication gates in `main.rs` that prevent `copilot-adapter start --daemon` from running interactive auth. Since auth validation happens before daemonization (while the parent still has terminal access), both daemon and foreground modes can safely use the same `run_auth()` interactive flow.

---

## Background

### Current State
- `main.rs` lines ~50-51: `if !has_token && is_daemon { exit(1) }` — refuses to auth in daemon mode
- `main.rs` lines ~74-75: `if invalid_token && is_daemon { exit(1) }` — refuses to re-auth in daemon mode
- Foreground mode: calls `run_auth()` successfully in both cases

### Target State
- Both modes call `run_auth()` when credentials are missing or invalid
- Auth check at lines 44-88 (before daemonization at line ~100/~140) — parent has terminal
- `--skip-auth` flag still available for non-interactive environments

---

## Goals and Non-Goals

### Goals

| ID | Goal | Success Criteria |
|----|------|------------------|
| G1 | Remove daemon auth gates | No `if is_daemon { exit(1) }` in auth validation path |
| G2 | Daemon mode auto-authenticates | `start --daemon` triggers device flow when needed |
| G3 | All existing tests pass | `cargo test` succeeds |

### Non-Goals

| ID | Non-Goal | Rationale |
|----|----------|-----------|
| NG1 | Change the auth flow itself | Only removing the gate, not modifying device flow |
| NG2 | Add new CLI flags | Existing `--skip-auth` is sufficient |

---

## Implementation Plan

### Epic 1: Remove Daemon Auth Gate

**Status:** Not Started

**Objective:** Remove the `if is_daemon` early-exit branches in `main.rs`.

#### Task 1.1: Remove no-token daemon gate

**File:** `src/main.rs` (MODIFIED)

**Before** (around line 48-54):
```rust
if !has_token {
    if is_daemon {
        eprintln!("No authentication credentials found.");
        eprintln!("Please run 'copilot-adapter auth' first, or use --skip-auth to bypass.");
        std::process::exit(1);
    }
    run_auth(false).await?;
}
```

**After:**
```rust
if !has_token {
    // Auth check runs before daemonization — parent has terminal access.
    run_auth(false).await?;
}
```

**Acceptance Criteria:**
- [ ] `if is_daemon` block removed (4 lines)
- [ ] Comment added explaining why this is safe

#### Task 1.2: Remove invalid-token daemon gate

**File:** `src/main.rs` (MODIFIED)

**Before** (around line 72-78):
```rust
Err(e) => {
    if is_daemon {
        eprintln!("Stored token is invalid or expired: {e}");
        eprintln!("Please run 'copilot-adapter auth --force' first, or use --skip-auth to bypass.");
        std::process::exit(1);
    }
    eprintln!("Stored token is invalid or expired: {e}");
    run_auth(true).await?;
}
```

**After:**
```rust
Err(e) => {
    eprintln!("Stored token is invalid or expired: {e}");
    // Re-auth runs before daemonization — parent has terminal access.
    run_auth(true).await?;
}
```

**Acceptance Criteria:**
- [ ] `if is_daemon` block removed (4 lines)
- [ ] Info message retained for user context
- [ ] Comment added explaining safety

---

### Epic 2: Testing

**Status:** Not Started

**Objective:** Verify the fix works and nothing regresses.

#### Task 2.1: Run existing test suite

```bash
cargo test
cargo clippy
```

**Acceptance Criteria:**
- [ ] All existing tests pass
- [ ] No clippy warnings

#### Task 2.2: Manual E2E verification

**Test 1 — Fresh daemon start:**
```bash
copilot-adapter logout          # Clear credentials
copilot-adapter start --daemon  # Should trigger auth flow, not exit
```
Expected: Device flow auth prompt appears, user can authenticate, then daemon starts.

**Test 2 — Skip-auth unchanged:**
```bash
copilot-adapter logout
copilot-adapter start --daemon --skip-auth
```
Expected: Server starts without auth (fails at first API request).

**Test 3 — Foreground unchanged:**
```bash
copilot-adapter logout
copilot-adapter start
```
Expected: Same behavior as before — triggers auth flow.

**Acceptance Criteria:**
- [ ] Test 1 passes (daemon auto-auth)
- [ ] Test 2 passes (skip-auth still works)
- [ ] Test 3 passes (foreground unchanged)

---

### Epic 3: Documentation

**Status:** Not Started

**Objective:** Update documentation to reflect the fix.

#### Task 3.1: Update backlog

**File:** `docs/design/BACKLOG.md` (MODIFIED)

Move the bug from "ToDo > Bugs" to "Done > Bugs":
```markdown
## Done

### Bugs
- Authentication flow in --daemon mode now leads through auth experience (same as foreground mode)
```

#### Task 3.2: Update CLAUDE.md

**File:** `CLAUDE.md` (MODIFIED)

Update the daemon mode note to reflect that daemon mode now supports interactive auth:
```markdown
- **Daemon mode auth**: `copilot-adapter start --daemon` runs interactive authentication
  in the parent process before daemonizing, since the auth check occurs before terminal
  detachment. Use `--skip-auth` to bypass authentication in non-interactive environments.
```

#### Task 3.3: Update e2e-testing.md

**File:** `docs/development/e2e-testing.md` (MODIFIED)

Add daemon auth test procedure:
```markdown
### Daemon Auto-Authentication
1. Run `copilot-adapter logout` to clear credentials
2. Run `copilot-adapter start --daemon`
3. Expected: Device flow auth prompt appears
4. Complete authorization in browser
5. Expected: Daemon starts successfully
6. Verify: `copilot-adapter status` shows running
```

**Acceptance Criteria:**
- [ ] Backlog updated
- [ ] CLAUDE.md updated
- [ ] E2E testing docs updated

---

## File Changes Summary

| File | Change | Epic | Description |
|------|--------|------|-------------|
| `src/main.rs` | Modified | Epic 1 | Remove ~8 lines of daemon auth gates |
| `docs/design/BACKLOG.md` | Modified | Epic 3 | Move bug to Done |
| `CLAUDE.md` | Modified | Epic 3 | Update daemon mode notes |
| `docs/development/e2e-testing.md` | Modified | Epic 3 | Add daemon auth test |

---

## Risk Assessment

| Risk | Impact | Probability | Mitigation |
|------|--------|-------------|------------|
| Auth blocks daemon startup in cron/CI | Low | Low | `--skip-auth` flag available |
| Regression in foreground auth | Low | Very Low | Same code path, just removing a guard |

---

## Success Criteria

1. **Daemon auto-auth works** — `start --daemon` without credentials triggers interactive auth
2. **Skip-auth preserved** — `--skip-auth` bypasses auth in both modes
3. **All tests pass** — `cargo test` succeeds
4. **No regressions** — Foreground mode behavior unchanged

---

## References

- [DAEMON-AUTH.design.md](./DAEMON-AUTH.design.md) — Design document
- [AUTO-AUTH-AND-ONBOARDING.plan.md](./archive/AUTO-AUTH-AND-ONBOARDING.plan.md) — Related auth improvements
- [BACKLOG.md](./BACKLOG.md) — Backlog item
