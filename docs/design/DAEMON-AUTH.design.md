# Daemon Authentication Fix — Design Document

**Status:** Proposed
**Date:** 2026-04-01
**Severity:** Medium
**Related:** [AUTO-AUTH-AND-ONBOARDING.plan.md](./archive/AUTO-AUTH-AND-ONBOARDING.plan.md)

---

## Executive Summary

When running `copilot-adapter start --daemon` without prior authentication, the adapter exits immediately with an error message instead of guiding the user through the interactive auth flow. This is unnecessary because the authentication check happens **before** daemonization — the parent process still has full terminal access. The fix removes the daemon-specific early-exit guards and lets both daemon and foreground modes use the same interactive authentication path.

---

## Context / Background

### Current State

The `start` command in `main.rs` (lines 44–88) validates authentication before starting the server. Two daemon-specific guards cause early exit:

1. **No token** (line ~50): If no GitHub token is stored and `is_daemon` is true, prints error and exits
2. **Invalid token** (line ~74): If stored token fails validation and `is_daemon` is true, prints error and exits

In foreground mode, both cases fall through to `run_auth()` which runs the interactive GitHub device flow.

### Target State

Both daemon and foreground modes should use the same `run_auth()` interactive flow when authentication is needed. The auth check occurs at lines 44–88, while daemonization happens much later (line ~100 for Unix, ~140 for Windows). The parent process retains full terminal access throughout the auth check phase.

---

## Problem Statement

**Observed behavior:**
- Running `copilot-adapter start --daemon` without credentials prints:
  ```
  No authentication credentials found.
  Please run 'copilot-adapter auth' first, or use --skip-auth to bypass.
  ```
  and exits with code 1.

**Expected behavior:**
- Running `copilot-adapter start --daemon` without credentials should launch the interactive device flow auth (display verification URI, wait for user authorization), then proceed to daemonize.

**Impact:**
- New users must run `auth` and `start --daemon` as two separate commands
- Inconsistent behavior between foreground and daemon modes
- Confusing UX — the adapter *could* auth interactively but refuses to

---

## Goals and Non-Goals

### Goals

| ID | Goal | Success Criteria |
|----|------|------------------|
| G1 | Daemon mode authenticates interactively when no token exists | `copilot-adapter start --daemon` without prior auth triggers device flow |
| G2 | Daemon mode re-authenticates when token is expired/invalid | Expired token triggers `run_auth(true)` instead of exiting |
| G3 | Maintain `--skip-auth` escape hatch | `--skip-auth` still bypasses all auth checks |
| G4 | No behavioral change for foreground mode | Foreground auth flow unchanged |

### Non-Goals

| ID | Non-Goal | Rationale |
|----|----------|-----------|
| NG1 | Auth inside the daemonized child process | Child has no terminal; auth must happen in parent |
| NG2 | Removing `--skip-auth` flag | Still needed for CI/CD and headless environments |
| NG3 | Changing the auth flow itself | Only removing the daemon gate, not modifying device flow |

---

## Research / Analysis

### Why the Original Design Refused Auth in Daemon Mode

The original reasoning was that daemon processes lack terminal access. This is **correct for the child process** after daemonization:
- Unix: After `daemon::daemonize()`, stdin/stdout/stderr → /dev/null
- Windows: Child spawned with `CREATE_NO_WINDOW | DETACHED_PROCESS`, null stdio

However, this reasoning **does not apply to the parent process** where auth validation occurs:

```
main.rs execution flow:
  Line 44-88:  Auth check ← PARENT PROCESS, HAS TERMINAL ✓
  Line 100:    daemon::daemonize() ← PARENT FORKS HERE
  Line 140:    daemon::spawn_background() ← WINDOWS SPAWNS HERE
```

The parent process retains full terminal access for the entire auth check phase. Interactive auth (device flow) works perfectly at this point.

### Validated Approach

The foreground mode already handles this case correctly — it calls `run_auth()` which displays the verification URI, offers to open the browser, and polls for authorization. The same code path works identically for daemon mode's parent process.

---

## Proposed Design

### Conceptual Change

Remove the `if is_daemon { exit(1) }` branches in the pre-start auth validation, letting daemon mode use the same `run_auth()` path as foreground mode.

### Before (main.rs lines ~44-88)

```rust
if !skip_auth {
    let store = storage::create_storage();
    let has_token = store.get_github_token().is_ok();

    if !has_token {
        if is_daemon {
            eprintln!("No authentication credentials found.");
            eprintln!("Please run 'copilot-adapter auth' first, or use --skip-auth to bypass.");
            std::process::exit(1);
        }
        run_auth(false).await?;
    }

    // ... token validation ...
    match manager.get_valid_token().await {
        Ok(_) => { /* good */ }
        Err(e) => {
            if is_daemon {
                eprintln!("Stored token is invalid or expired: {e}");
                eprintln!("Please run 'copilot-adapter auth --force' first, or use --skip-auth to bypass.");
                std::process::exit(1);
            }
            run_auth(true).await?;
        }
    }
}
```

### After

```rust
if !skip_auth {
    let store = storage::create_storage();
    let has_token = store.get_github_token().is_ok();

    if !has_token {
        // Both daemon and foreground: parent still has terminal access
        run_auth(false).await?;
    }

    // ... token validation ...
    match manager.get_valid_token().await {
        Ok(_) => { /* good */ }
        Err(e) => {
            eprintln!("Stored token is invalid or expired: {e}");
            // Both daemon and foreground: force re-auth
            run_auth(true).await?;
        }
    }
}
```

---

## Design Decisions

| Decision | Rationale |
|----------|-----------|
| Remove daemon auth gates entirely | Parent process has terminal access; no reason to block |
| Keep `--skip-auth` flag unchanged | Still needed for CI/CD and automated workflows |
| Don't add any new flags | Simpler UX — just works like foreground mode |
| Keep `eprintln` for expired token info | Useful context before re-auth prompt |

---

## File Changes Summary

| File | Change | Description |
|------|--------|-------------|
| `src/main.rs` | Modified | Remove `if is_daemon { exit(1) }` guards (~6 lines removed) |
| `tests/integration/daemon_tests.rs` | Modified | Update tests for new behavior |
| `docs/development/e2e-testing.md` | Modified | Update daemon auth test procedures |
| `docs/design/BACKLOG.md` | Modified | Move bug to Done section |
| `CLAUDE.md` | Modified | Update daemon mode notes |

---

## Testing Strategy

### Unit Tests
- No new unit tests needed (removing code, not adding)

### Integration Tests
- Update `daemon_tests.rs` to verify daemon startup doesn't exit when no auth

### Manual E2E Tests
1. **Fresh start with daemon**: `copilot-adapter start --daemon` without prior auth → should trigger device flow
2. **Expired token with daemon**: Invalidate token, then `start --daemon` → should re-auth
3. **Skip-auth unchanged**: `copilot-adapter start --daemon --skip-auth` → should skip auth as before
4. **Foreground unchanged**: `copilot-adapter start` → behavior identical to before

---

## Risk Assessment

| Risk | Impact | Probability | Mitigation |
|------|--------|-------------|------------|
| Auth flow blocks daemon startup | Low | Low | Already the case in foreground mode; `--skip-auth` available |
| Non-interactive terminal (e.g., cron) | Low | Low | `--skip-auth` flag + `wait_for_enter_or_timeout()` handles non-interactive stdin |
| Windows child receives auth prompt | Medium | Very Low | Auth happens in parent before spawn; child gets `--skip-auth` |

---

## Success Criteria

1. `copilot-adapter start --daemon` without prior auth → launches device flow → daemonizes after auth
2. `copilot-adapter start --daemon` with expired token → re-authenticates → daemonizes
3. `copilot-adapter start --daemon --skip-auth` → unchanged behavior
4. `copilot-adapter start` (foreground) → unchanged behavior
5. All existing tests pass

---

## References

- [AUTO-AUTH-AND-ONBOARDING.plan.md](./archive/AUTO-AUTH-AND-ONBOARDING.plan.md) — Related onboarding improvements
- `src/main.rs` — Entry point with auth validation flow
- `src/auth/device_flow.rs` — GitHub OAuth device flow
- `src/daemon/unix.rs` — Unix daemonization (double-fork)
- `src/daemon/windows.rs` — Windows background spawning
