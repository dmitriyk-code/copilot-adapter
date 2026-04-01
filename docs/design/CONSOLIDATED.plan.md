# Consolidated Implementation Plan — Backlog Items

**Status:** Not Started
**Date:** 2026-04-01
**Based on:** [BACKLOG.md](./BACKLOG.md), individual design documents
**Estimated Time:** 5–8 days (4 phases)

---

## Executive Summary

This plan consolidates the four outstanding backlog items into a single, sequenced implementation plan. The items share significant overlap in directory layout, storage APIs, CLI changes, and documentation updates — implementing them as separate plans would cause redundant work, API churn, and inconsistent intermediate states.

**Items consolidated:**
1. **DAEMON-AUTH** (Bug) — Remove daemon auth gate so `start --daemon` triggers interactive auth
2. **HOME-DIR-STATUS** (Nice-to-have) — Move runtime status to `~/.copilot-adapter/status.json`
3. **HOME-DIR-TOKEN** (Nice-to-have) — Move credentials to `~/.copilot-adapter/credentials.json`, file-first storage
4. **MULTI-INSTANCE-PROFILES** (Nice-to-have) — Named profiles for concurrent instances

**Key consolidation decisions:**
- Build the `~/.copilot-adapter/` directory structure once, with profiles in mind from the start
- Design parameterized APIs (`write_status_to(path)`, `FileStorage::with_path()`) from day one instead of creating them and then refactoring
- Single documentation update at the end covering all changes
- Phased testing: unit tests per phase, one integration/E2E pass at the end

**Design documents** (unchanged, for reference):
- [DAEMON-AUTH.design.md](./DAEMON-AUTH.design.md)
- [HOME-DIR-STATUS.design.md](./HOME-DIR-STATUS.design.md)
- [HOME-DIR-TOKEN.design.md](./HOME-DIR-TOKEN.design.md)
- [MULTI-INSTANCE-PROFILES.design.md](./MULTI-INSTANCE-PROFILES.design.md)

---

## Background

### Current State
- Daemon mode refuses interactive auth (bug)
- PID/port stored as two plain-text files in OS temp directory
- Credentials stored via OS keyring (primary) with encrypted file fallback at platform-specific paths
- Single instance only; no profile concept

### Target State
- Daemon and foreground modes behave identically for auth
- All state under `~/.copilot-adapter/profiles/<name>/` (status.json + credentials.json)
- File-based credential storage as default; keyring opt-in via `--use-keyring`
- Multiple concurrent instances via `--profile` / `-P` flag
- Default profile "default" preserves 100% backward compatibility

---

## Implementation Phases

The plan is organized into 4 phases with clear dependency boundaries. Each phase is independently shippable.

| Phase | Items Covered | Est. Days | Dependencies |
|-------|--------------|-----------|--------------|
| Phase 1 | DAEMON-AUTH bug fix | 0.5 | None |
| Phase 2 | HOME-DIR-STATUS + HOME-DIR-TOKEN | 2–3 | None (parallel with Phase 1) |
| Phase 3 | MULTI-INSTANCE-PROFILES | 2–3 | Phase 2 |
| Phase 4 | Integration testing + Documentation | 1 | Phases 1–3 |

---

## Phase 1: Daemon Authentication Fix (0.5 days)

**Objective:** Remove daemon-specific auth gates so `start --daemon` triggers interactive auth.

### Task 1.1: Remove no-token daemon gate

**File:** `src/main.rs` (MODIFIED)

**Before** (around line 48–54):
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

### Task 1.2: Remove invalid-token daemon gate

**File:** `src/main.rs` (MODIFIED)

**Before** (around line 72–78):
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

### Task 1.3: Verify

```bash
cargo test
cargo clippy
```

**Acceptance Criteria:**
- [ ] No `if is_daemon { exit(1) }` in auth validation path
- [ ] `start --daemon` triggers device flow when no credentials
- [ ] `--skip-auth` still bypasses auth in both modes
- [ ] All existing tests pass, no clippy warnings

---

## Phase 2: Home Directory Storage (2–3 days)

**Objective:** Move status and credentials to `~/.copilot-adapter/`, designing APIs that are profile-aware from the start (accepting path parameters) to avoid rework in Phase 3.

### Epic 2A: Status File Module

#### Task 2A.1: Create status module with parameterized API

**File:** `src/daemon/status.rs` (NEW)

Design the API to accept paths from the start. The convenience functions (no-path variants) will use a default path that Phase 3 replaces with profile resolution.

```rust
use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StatusFile {
    pub pid: u32,
    pub port: u16,
    pub started_at: String,  // ISO 8601
    pub version: String,
}

/// Returns the base directory: ~/.copilot-adapter/
/// Falls back to temp dir if home is not writable.
pub fn get_base_dir() -> PathBuf {
    if let Some(home) = dirs::home_dir() {
        let dir = home.join(".copilot-adapter");
        if std::fs::create_dir_all(&dir).is_ok() {
            return dir;
        }
    }
    std::env::temp_dir()
}

/// Default status file path (non-profile mode).
pub fn get_default_status_path() -> PathBuf {
    get_base_dir().join("status.json")
}

// --- Parameterized API (used directly in Phase 3) ---

pub fn write_status_to(path: &Path, port: u16) -> Result<()> {
    let status = StatusFile {
        pid: std::process::id(),
        port,
        started_at: chrono::Utc::now().to_rfc3339(),
        version: env!("CARGO_PKG_VERSION").to_string(),
    };
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, serde_json::to_string_pretty(&status)?)?;
    Ok(())
}

pub fn read_status_from(path: &Path) -> Option<StatusFile> {
    let content = std::fs::read_to_string(path).ok()?;
    serde_json::from_str(&content).ok()
}

pub fn remove_status_from(path: &Path) {
    let _ = std::fs::remove_file(path);
}

// --- Convenience wrappers (default path, used until Phase 3) ---

pub fn write_status(port: u16) -> Result<()> {
    write_status_to(&get_default_status_path(), port)
}

pub fn read_status() -> Option<StatusFile> {
    read_status_from(&get_default_status_path())
}

pub fn remove_status() {
    remove_status_from(&get_default_status_path());
}

// --- Running check with legacy fallback ---

pub fn is_running_from_status() -> Option<StatusFile> {
    // Check new location first
    if let Some(status) = read_status() {
        if super::process_exists(status.pid) {
            return Some(status);
        }
        remove_status();
    }
    // Legacy fallback: check temp dir PID file
    let pid_path = super::get_pid_path();
    if let Ok(content) = std::fs::read_to_string(&pid_path) {
        if let Ok(pid) = content.trim().parse::<u32>() {
            if super::process_exists(pid) {
                let port = super::read_port().unwrap_or(0);
                return Some(StatusFile {
                    pid,
                    port,
                    started_at: "unknown".to_string(),
                    version: "unknown".to_string(),
                });
            }
            let _ = std::fs::remove_file(&pid_path);
        }
    }
    None
}
```

**Acceptance Criteria:**
- [ ] StatusFile struct with serde derives
- [ ] Parameterized `*_to()` / `*_from()` API for Phase 3
- [ ] Convenience wrappers using default path
- [ ] Legacy backward compatibility in `is_running_from_status()`

#### Task 2A.2: Register module and update daemon/mod.rs

**File:** `src/daemon/mod.rs` (MODIFIED)

- Add `pub mod status;` and `pub use status::*;`
- Update `is_running()` to delegate to `is_running_from_status().map(|s| s.pid)`
- Update `read_port()` to check `read_status().map(|s| s.port)` first, then legacy
- Add `remove_all_status_files()` that cleans new + legacy files

#### Task 2A.3: Update daemon stop functions

**Files:** `src/daemon/unix.rs`, `src/daemon/windows.rs` (MODIFIED)

Replace `remove_pid_file(); remove_port_file();` with `super::remove_all_status_files();`

#### Task 2A.4: Update server.rs

**File:** `src/server.rs` (MODIFIED)

Replace `write_pid_file()` + `write_port_file(port)` with `write_status(port)`.
Replace cleanup calls with `remove_all_status_files()`.

#### Task 2A.5: Update main.rs Status command

**File:** `src/main.rs` (MODIFIED)

Use `is_running_from_status()` for richer output (PID, port, version, start time).

#### Task 2A.6: Unit tests for StatusFile

- Serialization/deserialization round-trip
- Directory resolution (home dir + fallback)
- Write/read/remove lifecycle
- Stale file cleanup (dead PID)
- Legacy PID file detection

---

### Epic 2B: File-First Credential Storage

#### Task 2B.1: Update FileStorage path and add migration

**File:** `src/storage/file.rs` (MODIFIED)

Change default path to `~/.copilot-adapter/credentials.json`. Add `get_legacy_credentials_path()` for old platform-specific paths. Add `migrate_if_needed()` that copies from old to new on first access.

Add `with_path(path: PathBuf)` constructor for Phase 3 profile support:

```rust
impl FileStorage {
    pub fn new() -> Self {
        let path = get_credentials_path();
        migrate_if_needed(&path);
        Self { path }
    }

    pub fn with_path(path: PathBuf) -> Self {
        Self { path }
    }
}
```

#### Task 2B.2: Flip storage priority

**File:** `src/storage/mod.rs` (MODIFIED)

Change `create_storage()` to accept `use_keyring: bool`. Default to file storage; only try keyring when `use_keyring` is true.

Design with profile support in mind:

```rust
pub fn create_storage(use_keyring: bool) -> Box<dyn TokenStorage + Send + Sync> {
    create_storage_with_path(get_credentials_path(), use_keyring)
}

pub fn create_storage_with_path(
    path: PathBuf,
    use_keyring: bool,
) -> Box<dyn TokenStorage + Send + Sync> {
    if use_keyring {
        if let Ok(ks) = keyring::KeyringStorage::new() {
            if ks.verify_available().unwrap_or(false) {
                tracing::info!("Using OS keyring for credential storage");
                return Box::new(ks);
            }
        }
        tracing::warn!("Keyring not available, falling back to file storage");
    }
    tracing::info!("Using file-based credential storage");
    Box::new(file::FileStorage::with_path(path))
}
```

#### Task 2B.3: Add --use-keyring CLI flag

**File:** `src/cli.rs` (MODIFIED)

Add `--use-keyring` to `Start` and `Auth` commands.

#### Task 2B.4: Wire through main.rs

**File:** `src/main.rs` (MODIFIED)

Update all `create_storage()` calls to pass `use_keyring`.

#### Task 2B.5: Unit tests for credential storage

- Path resolution returns `~/.copilot-adapter/credentials.json`
- Migration from old path
- `create_storage(false)` returns FileStorage
- `create_storage(true)` tries keyring first
- `with_path()` uses custom path

---

## Phase 3: Multi-Instance Profiles (2–3 days)

**Objective:** Introduce profile concept. Builds on the parameterized APIs from Phase 2.

### Epic 3A: Profile Data Model

#### Task 3A.1: Create profile types

**File:** `src/profile/types.rs` (NEW)

```rust
use anyhow::Result;
use std::path::PathBuf;

#[derive(Debug, Clone)]
pub struct Profile {
    pub name: String,
    pub dir: PathBuf,
}

impl Profile {
    pub fn status_path(&self) -> PathBuf { self.dir.join("status.json") }
    pub fn credentials_path(&self) -> PathBuf { self.dir.join("credentials.json") }
}

pub fn validate_profile_name(name: &str) -> Result<()> {
    if name.is_empty() || name.len() > 64 {
        anyhow::bail!("Profile name must be 1-64 characters");
    }
    if !name.chars().all(|c| c.is_alphanumeric() || c == '-' || c == '_') {
        anyhow::bail!("Profile name may only contain letters, digits, dash, underscore");
    }
    Ok(())
}
```

#### Task 3A.2: Create ProfileManager

**File:** `src/profile/mod.rs` (NEW)

Implement `new()`, `get()`, `list()`, `create()`, `delete()`, `find_by_port()`, `check_port_conflict()`.

Uses `get_base_dir()` from `daemon::status` for the `~/.copilot-adapter/` root.

#### Task 3A.3: Register module

**File:** `src/lib.rs` (MODIFIED) — Add `pub mod profile;`

#### Task 3A.4: Unit tests for profile model

- Name validation (valid, invalid, empty, 65 chars, special chars)
- ProfileManager CRUD
- Port conflict detection
- Default profile behavior

---

### Epic 3B: Profile-Scoped Storage and Status

#### Task 3B.1: Wire ProfileManager into storage

**File:** `src/storage/mod.rs` (MODIFIED)

Add convenience function that takes a Profile:

```rust
pub fn create_storage_for_profile(
    profile: &Profile,
    use_keyring: bool,
) -> Box<dyn TokenStorage + Send + Sync> {
    create_storage_with_path(profile.credentials_path(), use_keyring)
}
```

No API churn — `create_storage_with_path()` was built in Phase 2.

#### Task 3B.2: Wire ProfileManager into status

No changes to `daemon/status.rs` — the `write_status_to()`, `read_status_from()`, `remove_status_from()` functions already accept paths. Callers just pass `profile.status_path()`.

---

### Epic 3C: CLI Changes

#### Task 3C.1: Add --profile and --all flags

**File:** `src/cli.rs` (MODIFIED)

- Add `--profile` / `-P` with default "default" to Start, Stop, Status, Auth, Logout
- Add `--all` to Stop and Status
- Add `Profiles` subcommand with `List`, `Create { name }`, `Delete { name }`

#### Task 3C.2: Update main.rs — Profile resolution

**File:** `src/main.rs` (MODIFIED)

At the top of command handling:
```rust
let pm = ProfileManager::new();
let profile = pm.get(&profile_name)?;
```

#### Task 3C.3: Update main.rs — Profile-scoped start

Use `create_storage_for_profile(&profile, use_keyring)` for auth.
Use `write_status_to(&profile.status_path(), port)` for status.
Check port conflicts via `pm.check_port_conflict(port, &profile.name)`.

#### Task 3C.4: Update main.rs — Profile-scoped stop/status

`--all` iterates `pm.list()` and operates on each. Single profile uses named profile.

#### Task 3C.5: Update main.rs — Profiles subcommand handler

Handle `profiles list`, `profiles create <name>`, `profiles delete <name>`.

---

### Epic 3D: Migration

#### Task 3D.1: Auto-migration on first run

If `~/.copilot-adapter/profiles/` doesn't exist but `~/.copilot-adapter/status.json` or `~/.copilot-adapter/credentials.json` does:
1. Create `~/.copilot-adapter/profiles/default/`
2. Move `status.json` and `credentials.json` into it
3. Log the migration

#### Task 3D.2: Legacy temp dir migration

Check temp dir PID file and migrate to default profile status. Remove legacy convenience wrappers that are no longer needed (or keep as thin delegates for backward compat in tests).

---

## Phase 4: Testing and Documentation (1 day)

**Objective:** Comprehensive integration testing and unified documentation update.

### Epic 4A: Integration Tests

#### Task 4A.1: Daemon auth integration test

Verify `start --daemon` without credentials doesn't exit.

#### Task 4A.2: Status file lifecycle test

Write → is_running → remove → not running. Verify `~/.copilot-adapter/status.json`.

#### Task 4A.3: Credential storage lifecycle test

Auth → store → restart → load from new path. Migration from old path.

#### Task 4A.4: Profile lifecycle test

Create profile → auth → start → status → stop → delete.

#### Task 4A.5: Multi-instance test

Two profiles simultaneously on different ports. Port conflict rejection. `--all` operations.

#### Task 4A.6: Migration test

Single-instance data → default profile migration. Legacy temp dir PID file handling.

---

### Epic 4B: Manual E2E Tests

#### Task 4B.1: Daemon auth E2E

```bash
copilot-adapter logout
copilot-adapter start --daemon   # Should trigger auth flow
copilot-adapter status           # Should show running
copilot-adapter stop
```

#### Task 4B.2: Home directory storage E2E

```bash
copilot-adapter auth
# Verify: ~/.copilot-adapter/credentials.json exists
copilot-adapter start
# Verify: ~/.copilot-adapter/status.json exists with PID, port, version, started_at
copilot-adapter status  # Shows rich output
copilot-adapter stop
```

#### Task 4B.3: Multi-instance E2E

```bash
copilot-adapter profiles create work
copilot-adapter auth -P work
copilot-adapter start -P work -p 8080
copilot-adapter status --all     # Shows default + work
copilot-adapter stop --all       # Stops all
copilot-adapter profiles delete work
```

---

### Epic 4C: Documentation (single pass)

#### Task 4C.1: Update CLAUDE.md

- Add `src/daemon/status.rs` and `src/profile/` to project structure
- Add development notes for: daemon auth, home directory storage, profiles, `--use-keyring`
- Update CLI commands table with `--profile`, `--use-keyring`, `profiles` subcommand

#### Task 4C.2: Update docs/e2e-testing.md

Add test procedures for daemon auth, home dir storage, and multi-instance profiles.

#### Task 4C.3: Update BACKLOG.md

Move all four items from "ToDo" to "Done":
```markdown
## Done

### Bugs
- Authentication flow in --daemon mode now leads through auth experience (same as foreground)

### Features
- Runtime status stored in ~/.copilot-adapter/status.json (with legacy fallback)
- Credentials stored in ~/.copilot-adapter/credentials.json by default; --use-keyring for OS keyring
- Multi-instance profiles via --profile / -P flag; profiles subcommand for management
```

---

## File Changes Summary

| File | Change | Phase | Description |
|------|--------|-------|-------------|
| `src/main.rs` | Modified | 1, 2, 3 | Remove daemon auth gates; richer status; profile resolution |
| `src/daemon/status.rs` | **New** | 2 | StatusFile struct, parameterized read/write/remove, legacy compat |
| `src/daemon/mod.rs` | Modified | 2 | Register status module, delegate to new functions |
| `src/daemon/unix.rs` | Modified | 2 | Use remove_all_status_files() |
| `src/daemon/windows.rs` | Modified | 2 | Use remove_all_status_files() |
| `src/server.rs` | Modified | 2 | Use write_status()/remove_all_status_files() |
| `src/storage/file.rs` | Modified | 2 | New default path, migration, with_path() constructor |
| `src/storage/mod.rs` | Modified | 2, 3 | use_keyring param, create_storage_with_path(), profile helper |
| `src/cli.rs` | Modified | 2, 3 | --use-keyring, --profile, --all, Profiles subcommand |
| `src/profile/mod.rs` | **New** | 3 | ProfileManager |
| `src/profile/types.rs` | **New** | 3 | Profile struct, name validation |
| `src/lib.rs` | Modified | 3 | Add `pub mod profile;` |
| `CLAUDE.md` | Modified | 4 | Project structure, dev notes, CLI table |
| `docs/design/BACKLOG.md` | Modified | 4 | Move all items to Done |
| `docs/e2e-testing.md` | Modified | 4 | New test procedures |
| `tests/unit/status_tests.rs` | **New** | 2 | Status file unit tests |
| `tests/unit/storage_tests.rs` | Modified | 2 | Credential storage tests |
| `tests/unit/profile_tests.rs` | **New** | 3 | Profile model unit tests |
| `tests/integration/daemon_tests.rs` | Modified | 4 | Updated for new APIs |
| `tests/integration/profile_tests.rs` | **New** | 4 | Profile lifecycle + multi-instance tests |

---

## Overlap Eliminated

The following redundancies from the individual plans are resolved:

| Redundancy | Individual Plans | Consolidated Approach |
|------------|-----------------|----------------------|
| Create `~/.copilot-adapter/` directory | HOME-DIR-STATUS + HOME-DIR-TOKEN both create it | Created once in `get_base_dir()` (Phase 2A) |
| `create_storage()` API change | HOME-DIR-TOKEN adds `use_keyring` param; MULTI-INSTANCE-PROFILES adds `for_profile()` | Build `create_storage_with_path(path, use_keyring)` once (Phase 2B), add `for_profile()` wrapper (Phase 3B) |
| Status functions API | HOME-DIR-STATUS creates fixed-path functions; MULTI-INSTANCE-PROFILES refactors to accept paths | Build parameterized `*_to()`/`*_from()` API from day one (Phase 2A) |
| `remove_all_status_files()` | HOME-DIR-STATUS creates it; MULTI-INSTANCE-PROFILES changes to profile-scoped | Build cleanup that handles both legacy and profile paths |
| Documentation updates | Each plan has "Update CLAUDE.md", "Update BACKLOG.md", "Update e2e-testing.md" | Single documentation pass at the end (Phase 4C) |
| Testing epics | 4 separate testing epics | Phased: unit tests with each phase, integration + E2E once at end |

---

## Dependencies

### New Dependencies
- `chrono` — already in Cargo.toml (for `started_at` timestamp)
- `dirs` — already in Cargo.toml (for home directory resolution)

### Sequencing

```
Phase 1 (daemon auth)  ──────────────────────────┐
                                                   ├──→ Phase 4 (testing + docs)
Phase 2 (home dir storage) ──→ Phase 3 (profiles) ┘
```

Phases 1 and 2 can run in parallel. Phase 3 depends on Phase 2. Phase 4 depends on all.

---

## Risk Assessment

| Risk | Impact | Probability | Mitigation |
|------|--------|-------------|------------|
| Home dir not writable | Medium | Low | Fallback to temp dir in get_base_dir() |
| Existing keyring users lose credentials | Medium | Medium | Migration logic; keyring fallback if file empty |
| Breaking backward compat with profiles | High | Low | Default profile = existing behavior; extensive testing |
| Port conflicts between profiles | Medium | Medium | Explicit check at startup with clear error |
| Legacy PID files left behind | Low | Medium | Backward compat in is_running_from_status() |
| Auth blocking daemon in cron/CI | Low | Low | --skip-auth flag available |
| Migration race condition | Low | Very Low | Idempotent copy operations |

---

## Success Criteria

1. **Daemon auth works** — `start --daemon` without credentials triggers interactive auth
2. **Status in home dir** — `~/.copilot-adapter/status.json` created on start with PID, port, version, started_at
3. **Credentials in home dir** — `~/.copilot-adapter/credentials.json` as default; `--use-keyring` for OS keyring
4. **Profiles work** — `start -P work -p 8080` runs a second instance; `status --all` shows both
5. **Backward compatible** — All commands work without any flags (default profile on port 6767)
6. **Migration seamless** — Old temp dir PID files and platform-specific credential files auto-discovered
7. **All tests pass** — `cargo test` succeeds with zero failures

---

## References

- [DAEMON-AUTH.design.md](./DAEMON-AUTH.design.md)
- [HOME-DIR-STATUS.design.md](./HOME-DIR-STATUS.design.md)
- [HOME-DIR-TOKEN.design.md](./HOME-DIR-TOKEN.design.md)
- [MULTI-INSTANCE-PROFILES.design.md](./MULTI-INSTANCE-PROFILES.design.md)
- [BACKLOG.md](./BACKLOG.md)
