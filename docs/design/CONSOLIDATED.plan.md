# Consolidated Implementation Plan — Backlog Items

**Status:** Not Started
**Date:** 2026-04-01
**Based on:** [BACKLOG.md](./BACKLOG.md), individual design documents
**Prerequisite:** None — Epics 1, 2, and 3 can start immediately
**Estimated Time:** 5–8 days (10 epics)

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
- Phased testing: unit tests per epic, one integration/E2E pass at the end

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

## Goals and Non-Goals

### Goals

| ID | Goal | Success Criteria |
|----|------|------------------|
| G1 | Fix daemon auth gate — `start --daemon` triggers interactive auth when no credentials | `start --daemon` without credentials opens device flow instead of exiting |
| G2 | Move runtime status to `~/.copilot-adapter/status.json` | `status.json` created on start with PID, port, version, started_at; legacy temp files auto-detected |
| G3 | Move credentials to `~/.copilot-adapter/credentials.json` (file-first) | Default storage is file-based; `--use-keyring` opts into OS keyring; migration from old paths is seamless |
| G4 | Support multiple concurrent instances via named profiles | `start -P work -p 8080` runs a second instance; `status --all` shows all profiles |
| G5 | 100% backward compatibility with no-flag usage | All commands work without flags using "default" profile on port 6767 |

### Non-Goals

| ID | Non-Goal | Rationale |
|----|----------|-----------|
| NG1 | Remote/network-accessible multi-instance coordination | Localhost-only security model; profiles are local file-based |
| NG2 | Automatic profile switching based on Git context | Too complex for initial implementation; can be added later |
| NG3 | Credential encryption changes | Current AES-256-GCM encryption is sufficient; only path changes |
| NG4 | Keyring removal | Keyring remains available via `--use-keyring`; just no longer default |

---

## Implementation Plan

The plan is organized into 10 epics with clear dependency boundaries.

| Epic | Name | Est. Days | Dependencies |
|------|------|-----------|--------------|
| Epic 1 | Daemon Authentication Fix | 0.5 | None |
| Epic 2 | Status File Module | 1–1.5 | None |
| Epic 3 | File-First Credential Storage | 1–1.5 | None |
| Epic 4 | Profile Data Model | 1 | Epics 2–3 |
| Epic 5 | Profile-Scoped Storage and Status | 0.5 | Epics 2–4 |
| Epic 6 | CLI Changes | 1 | Epics 4–5 |
| Epic 7 | Migration | 0.5 | Epics 2–4 |
| Epic 8 | Integration Tests | 0.5 | Epics 1–7 |
| Epic 9 | Manual E2E Tests | 0.25 | Epics 1–7 |
| Epic 10 | Documentation | 0.25 | Epics 1–7 |

---

## Epic 1: Daemon Authentication Fix (0.5 days)

**Status:** DONE

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
- [x] No `if is_daemon { exit(1) }` in auth validation path
- [x] `start --daemon` triggers device flow when no credentials
- [x] `--skip-auth` still bypasses auth in both modes
- [x] All existing tests pass, no clippy warnings

---

## Epic 2: Status File Module (1–1.5 days)

**Status:** Not Started

**Objective:** Create `~/.copilot-adapter/status.json` with parameterized APIs designed for profile support from the start.

### Task 2.1: Create status module with parameterized API

**File:** `src/daemon/status.rs` (NEW)

Design the API to accept paths from the start. The convenience functions (no-path variants) will use a default path that Epic 4 replaces with profile resolution.

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

// --- Parameterized API (used directly in Epic 4+) ---

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

// --- Convenience wrappers (default path, used until Epic 4) ---

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
- [ ] Parameterized `*_to()` / `*_from()` API for Epic 4+
- [ ] Convenience wrappers using default path
- [ ] Legacy backward compatibility in `is_running_from_status()`

### Task 2.2: Register module and update daemon/mod.rs

**File:** `src/daemon/mod.rs` (MODIFIED)

- Add `pub mod status;` and `pub use status::*;`
- Update `is_running()` to delegate to `is_running_from_status().map(|s| s.pid)`
- Update `read_port()` to check `read_status().map(|s| s.port)` first, then legacy
- Add `remove_all_status_files()` that cleans new + legacy files

### Task 2.3: Update daemon stop functions

**Files:** `src/daemon/unix.rs`, `src/daemon/windows.rs` (MODIFIED)

Replace `remove_pid_file(); remove_port_file();` with `super::remove_all_status_files();`

### Task 2.4: Update server.rs

**File:** `src/server.rs` (MODIFIED)

Replace `write_pid_file()` + `write_port_file(port)` with `write_status(port)`.
Replace cleanup calls with `remove_all_status_files()`.

### Task 2.5: Update main.rs Status command

**File:** `src/main.rs` (MODIFIED)

Use `is_running_from_status()` for richer output (PID, port, version, start time).

### Task 2.6: Unit tests for StatusFile

- Serialization/deserialization round-trip
- Directory resolution (home dir + fallback)
- Write/read/remove lifecycle
- Stale file cleanup (dead PID)
- Legacy PID file detection

---

## Epic 3: File-First Credential Storage (1–1.5 days)

**Status:** Not Started

**Objective:** Move credentials to `~/.copilot-adapter/credentials.json` as the default, with keyring as opt-in.

### Task 3.1: Update FileStorage path and add migration

**File:** `src/storage/file.rs` (MODIFIED)

Change default path to `~/.copilot-adapter/credentials.json`. Add `get_legacy_credentials_path()` for old platform-specific paths. Add `migrate_if_needed()` that copies from old to new on first access.

Add `with_path(path: PathBuf)` constructor for profile support in Epic 5:

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

### Task 3.2: Flip storage priority

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

### Task 3.3: Add --use-keyring CLI flag

**File:** `src/cli.rs` (MODIFIED)

Add `--use-keyring` to `Start` and `Auth` commands.

### Task 3.4: Wire through main.rs

**File:** `src/main.rs` (MODIFIED)

Update all `create_storage()` calls to pass `use_keyring`.

### Task 3.5: Unit tests for credential storage

- Path resolution returns `~/.copilot-adapter/credentials.json`
- Migration from old path
- `create_storage(false)` returns FileStorage
- `create_storage(true)` tries keyring first
- `with_path()` uses custom path

---

## Epic 4: Profile Data Model (1 day)

**Status:** Not Started

**Objective:** Introduce profile types and ProfileManager. Builds on the parameterized APIs from Epics 2–3.

### Task 4.1: Create profile types

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

### Task 4.2: Create ProfileManager

**File:** `src/profile/mod.rs` (NEW)

Implement `new()`, `get()`, `list()`, `create()`, `delete()`, `find_by_port()`, `check_port_conflict()`.

Uses `get_base_dir()` from `daemon::status` for the `~/.copilot-adapter/` root.

### Task 4.3: Register module

**File:** `src/lib.rs` (MODIFIED) — Add `pub mod profile;`

### Task 4.4: Unit tests for profile model

- Name validation (valid, invalid, empty, 65 chars, special chars)
- ProfileManager CRUD
- Port conflict detection
- Default profile behavior

---

## Epic 5: Profile-Scoped Storage and Status (0.5 days)

**Status:** Not Started

**Objective:** Wire ProfileManager into the storage and status APIs built in Epics 2–3.

### Task 5.1: Wire ProfileManager into storage

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

No API churn — `create_storage_with_path()` was built in Epic 3.

### Task 5.2: Wire ProfileManager into status

No changes to `daemon/status.rs` — the `write_status_to()`, `read_status_from()`, `remove_status_from()` functions already accept paths. Callers just pass `profile.status_path()`.

---

## Epic 6: CLI Changes (1 day)

**Status:** Not Started

**Objective:** Add `--profile`, `--all`, and `profiles` subcommand to the CLI.

### Task 6.1: Add --profile and --all flags

**File:** `src/cli.rs` (MODIFIED)

- Add `--profile` / `-P` with default "default" to Start, Stop, Status, Auth, Logout
- Add `--all` to Stop and Status
- Add `Profiles` subcommand with `List`, `Create { name }`, `Delete { name }`

### Task 6.2: Update main.rs — Profile resolution

**File:** `src/main.rs` (MODIFIED)

At the top of command handling:
```rust
let pm = ProfileManager::new();
let profile = pm.get(&profile_name)?;
```

### Task 6.3: Update main.rs — Profile-scoped start

Use `create_storage_for_profile(&profile, use_keyring)` for auth.
Use `write_status_to(&profile.status_path(), port)` for status.
Check port conflicts via `pm.check_port_conflict(port, &profile.name)`.

### Task 6.4: Update main.rs — Profile-scoped stop/status

`--all` iterates `pm.list()` and operates on each. Single profile uses named profile.

### Task 6.5: Update main.rs — Profiles subcommand handler

Handle `profiles list`, `profiles create <name>`, `profiles delete <name>`.

---

## Epic 7: Migration (0.5 days)

**Status:** Not Started

**Objective:** Auto-migrate from flat directory and legacy temp files to profile-based layout.

### Task 7.1: Auto-migration on first run

If `~/.copilot-adapter/profiles/` doesn't exist but `~/.copilot-adapter/status.json` or `~/.copilot-adapter/credentials.json` does:
1. Create `~/.copilot-adapter/profiles/default/`
2. Move `status.json` and `credentials.json` into it
3. Log the migration

### Task 7.2: Legacy temp dir migration

Check temp dir PID file and migrate to default profile status. Remove legacy convenience wrappers that are no longer needed (or keep as thin delegates for backward compat in tests).

---

## Epic 8: Integration Tests (0.5 days)

**Status:** Not Started

**Objective:** Automated integration tests covering all new functionality.

### Task 8.1: Daemon auth integration test

Verify `start --daemon` without credentials doesn't exit.

### Task 8.2: Status file lifecycle test

Write → is_running → remove → not running. Verify `~/.copilot-adapter/status.json`.

### Task 8.3: Credential storage lifecycle test

Auth → store → restart → load from new path. Migration from old path.

### Task 8.4: Profile lifecycle test

Create profile → auth → start → status → stop → delete.

### Task 8.5: Multi-instance test

Two profiles simultaneously on different ports. Port conflict rejection. `--all` operations.

### Task 8.6: Migration test

Single-instance data → default profile migration. Legacy temp dir PID file handling.

---

## Epic 9: Manual E2E Tests (0.25 days)

**Status:** Not Started

**Objective:** Document and execute manual end-to-end test procedures.

### Task 9.1: Daemon auth E2E

```bash
copilot-adapter logout
copilot-adapter start --daemon   # Should trigger auth flow
copilot-adapter status           # Should show running
copilot-adapter stop
```

### Task 9.2: Home directory storage E2E

```bash
copilot-adapter auth
# Verify: ~/.copilot-adapter/credentials.json exists
copilot-adapter start
# Verify: ~/.copilot-adapter/status.json exists with PID, port, version, started_at
copilot-adapter status  # Shows rich output
copilot-adapter stop
```

### Task 9.3: Multi-instance E2E

```bash
copilot-adapter profiles create work
copilot-adapter auth -P work
copilot-adapter start -P work -p 8080
copilot-adapter status --all     # Shows default + work
copilot-adapter stop --all       # Stops all
copilot-adapter profiles delete work
```

---

## Epic 10: Documentation (0.25 days)

**Status:** Not Started

**Objective:** Update all relevant documentation in a single pass.

### Task 10.1: Update CLAUDE.md

- Add `src/daemon/status.rs` and `src/profile/` to project structure
- Add development notes for: daemon auth, home directory storage, profiles, `--use-keyring`
- Update CLI commands table with `--profile`, `--use-keyring`, `profiles` subcommand

### Task 10.2: Update docs/e2e-testing.md

Add test procedures for daemon auth, home dir storage, and multi-instance profiles.

### Task 10.3: Update BACKLOG.md

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

## Requirements

### Functional Requirements

| ID | Requirement | Source | Epic |
|----|-------------|--------|------|
| FR1 | `start --daemon` triggers interactive auth when no credentials exist | DAEMON-AUTH.design.md | Epic 1 |
| FR2 | `start --daemon` triggers re-auth when stored token is invalid/expired | DAEMON-AUTH.design.md | Epic 1 |
| FR3 | `--skip-auth` bypasses auth in both foreground and daemon modes | DAEMON-AUTH.design.md | Epic 1 |
| FR4 | Runtime status written to `~/.copilot-adapter/status.json` with PID, port, version, started_at | HOME-DIR-STATUS.design.md | Epic 2 |
| FR5 | Legacy temp dir PID files detected and used as fallback | HOME-DIR-STATUS.design.md | Epic 2 |
| FR6 | `status` command shows rich output (PID, port, version, start time) | HOME-DIR-STATUS.design.md | Epic 2 |
| FR7 | Credentials stored in `~/.copilot-adapter/credentials.json` by default | HOME-DIR-TOKEN.design.md | Epic 3 |
| FR8 | `--use-keyring` flag opts into OS keyring storage | HOME-DIR-TOKEN.design.md | Epic 3 |
| FR9 | Migration from old platform-specific credential paths on first access | HOME-DIR-TOKEN.design.md | Epic 3 |
| FR10 | `--profile` / `-P` flag selects a named profile (default: "default") | MULTI-INSTANCE-PROFILES.design.md | Epic 6 |
| FR11 | Multiple concurrent instances on different ports via profiles | MULTI-INSTANCE-PROFILES.design.md | Epic 6 |
| FR12 | `profiles list/create/delete` subcommand for profile management | MULTI-INSTANCE-PROFILES.design.md | Epic 6 |
| FR13 | `--all` flag on stop/status operates on all profiles | MULTI-INSTANCE-PROFILES.design.md | Epic 6 |
| FR14 | Port conflict detection across profiles at startup | MULTI-INSTANCE-PROFILES.design.md | Epic 4 |
| FR15 | Auto-migration from flat `~/.copilot-adapter/` to `profiles/default/` | MULTI-INSTANCE-PROFILES.design.md | Epic 7 |

### Non-Functional Requirements

| ID | Requirement | Target | Epic |
|----|-------------|--------|------|
| NFR1 | Backward compatibility | All commands work without flags using "default" profile on port 6767 | All |
| NFR2 | Graceful degradation on unwritable home dir | Fall back to temp dir | Epic 2 |
| NFR3 | Seamless credential migration | Zero user intervention; old credentials discovered automatically | Epic 3 |
| NFR4 | Profile name validation | 1-64 chars, alphanumeric + dash + underscore | Epic 4 |
| NFR5 | Idempotent migration operations | Safe to run multiple times without data loss | Epic 7 |

---

## File Changes Summary

| File | Change | Epic | Description |
|------|--------|------|-------------|
| `src/main.rs` | Modified | 1, 2, 3, 6 | Remove daemon auth gates; richer status; profile resolution |
| `src/daemon/status.rs` | **New** | 2 | StatusFile struct, parameterized read/write/remove, legacy compat |
| `src/daemon/mod.rs` | Modified | 2 | Register status module, delegate to new functions |
| `src/daemon/unix.rs` | Modified | 2 | Use remove_all_status_files() |
| `src/daemon/windows.rs` | Modified | 2 | Use remove_all_status_files() |
| `src/server.rs` | Modified | 2 | Use write_status()/remove_all_status_files() |
| `src/storage/file.rs` | Modified | 3 | New default path, migration, with_path() constructor |
| `src/storage/mod.rs` | Modified | 3, 5 | use_keyring param, create_storage_with_path(), profile helper |
| `src/cli.rs` | Modified | 3, 6 | --use-keyring, --profile, --all, Profiles subcommand |
| `src/profile/mod.rs` | **New** | 4 | ProfileManager |
| `src/profile/types.rs` | **New** | 4 | Profile struct, name validation |
| `src/lib.rs` | Modified | 4 | Add `pub mod profile;` |
| `CLAUDE.md` | Modified | 10 | Project structure, dev notes, CLI table |
| `docs/design/BACKLOG.md` | Modified | 10 | Move all items to Done |
| `docs/e2e-testing.md` | Modified | 10 | New test procedures |
| `tests/unit/status_tests.rs` | **New** | 2 | Status file unit tests |
| `tests/unit/storage_tests.rs` | Modified | 3 | Credential storage tests |
| `tests/unit/profile_tests.rs` | **New** | 4 | Profile model unit tests |
| `tests/integration/daemon_tests.rs` | Modified | 8 | Updated for new APIs |
| `tests/integration/profile_tests.rs` | **New** | 8 | Profile lifecycle + multi-instance tests |

---

## Testing Strategy

### Test Coverage

| Component | Unit Tests | Integration Tests | E2E Tests |
|-----------|------------|-------------------|-----------|
| Daemon auth fix | N/A | Epic 8 (Task 8.1) | Epic 9 (Task 9.1) |
| StatusFile module | Epic 2 (Task 2.6) | Epic 8 (Task 8.2) | Epic 9 (Task 9.2) |
| Credential storage | Epic 3 (Task 3.5) | Epic 8 (Task 8.3) | Epic 9 (Task 9.2) |
| Profile model | Epic 4 (Task 4.4) | Epic 8 (Task 8.4) | Epic 9 (Task 9.3) |
| Multi-instance | N/A | Epic 8 (Task 8.5) | Epic 9 (Task 9.3) |
| Migration | N/A | Epic 8 (Task 8.6) | N/A |

### Test Files

| File | Type | Coverage |
|------|------|----------|
| `tests/unit/status_tests.rs` | Unit | StatusFile CRUD, directory resolution, legacy fallback |
| `tests/unit/storage_tests.rs` | Unit | Credential path, migration, storage factory |
| `tests/unit/profile_tests.rs` | Unit | Profile name validation, CRUD, port conflicts |
| `tests/integration/daemon_tests.rs` | Integration | Daemon auth, status lifecycle, credential lifecycle |
| `tests/integration/profile_tests.rs` | Integration | Profile lifecycle, multi-instance, migration |
| `docs/e2e-testing.md` | Manual E2E | Daemon auth, home dir storage, multi-instance workflows |

---

## Dependencies

### External Dependencies

| Dependency | Version | Purpose | Epic |
|------------|---------|---------|------|
| `chrono` | (existing) | ISO 8601 timestamp for `started_at` in StatusFile | Epic 2 |
| `dirs` | (existing) | Home directory resolution for `~/.copilot-adapter/` | Epic 2 |
| `serde` / `serde_json` | (existing) | StatusFile serialization/deserialization | Epic 2 |
| `clap` | (existing) | CLI flags: `--use-keyring`, `--profile`, `--all`, `profiles` subcommand | Epics 3, 6 |

**Cargo.toml changes:** None — all dependencies already in Cargo.toml.

### Internal Dependencies

| Module | Required By | Status |
|--------|-------------|--------|
| `src/daemon/mod.rs` | Epic 2 (status module registration) | ✅ Exists |
| `src/daemon/status.rs` | Epics 2, 5 (status file management) | 🚧 Will create |
| `src/storage/file.rs` | Epic 3 (path change + migration) | ✅ Exists |
| `src/storage/mod.rs` | Epics 3, 5 (storage factory) | ✅ Exists |
| `src/profile/types.rs` | Epic 4 (profile data model) | 🚧 Will create |
| `src/profile/mod.rs` | Epics 4–7 (profile manager) | 🚧 Will create |
| `src/cli.rs` | Epics 3, 6 (new CLI flags) | ✅ Exists |
| `src/main.rs` | Epics 1–3, 6 (wiring changes) | ✅ Exists |

### Sequencing

```
Epic 1 (daemon auth)  ──────────────────────────────────────┐
                                                              ├──→ Epics 8–10 (testing + docs)
Epics 2–3 (home dir storage) ──→ Epics 4–7 (profiles) ──────┘
```

Epics 1, 2, and 3 can run in parallel. Epics 4–7 depend on Epics 2–3. Epics 8–10 depend on all.

---

## Risk Assessment

| Risk | Impact | Probability | Mitigation | Epic |
|------|--------|-------------|------------|------|
| Home dir not writable | Medium | Low | Fallback to temp dir in get_base_dir() | Epic 2 |
| Existing keyring users lose credentials | Medium | Medium | Migration logic; keyring fallback if file empty | Epic 3 |
| Breaking backward compat with profiles | High | Low | Default profile = existing behavior; extensive testing | Epic 6 |
| Port conflicts between profiles | Medium | Medium | Explicit check at startup with clear error | Epic 4 |
| Legacy PID files left behind | Low | Medium | Backward compat in is_running_from_status() | Epic 2 |
| Auth blocking daemon in cron/CI | Low | Low | --skip-auth flag available | Epic 1 |
| Migration race condition | Low | Very Low | Idempotent copy operations | Epic 7 |

---

## Success Criteria

1. **Daemon auth works** — `start --daemon` without credentials triggers interactive auth (Epic 1)
2. **Status in home dir** — `~/.copilot-adapter/status.json` created on start with PID, port, version, started_at (Epic 2)
3. **Credentials in home dir** — `~/.copilot-adapter/credentials.json` as default; `--use-keyring` for OS keyring (Epic 3)
4. **Profiles work** — `start -P work -p 8080` runs a second instance; `status --all` shows both (Epics 4–6)
5. **Backward compatible** — All commands work without any flags (default profile on port 6767) (All)
6. **Migration seamless** — Old temp dir PID files and platform-specific credential files auto-discovered (Epics 2, 3, 7)
7. **All tests pass** — Unit, integration, and E2E tests pass with zero failures (Epic 8)
8. **Documentation complete** — CLAUDE.md, e2e-testing.md, and BACKLOG.md updated (Epic 10)

---

## Rollout / Migration Plan

### Epics 1–3: Core Changes
- [ ] Remove daemon auth gates in `src/main.rs`
- [ ] Implement StatusFile module with parameterized API
- [ ] Implement file-first credential storage with migration
- [ ] Unit tests for status and credential storage
- [ ] Code review

### Epics 4–7: Profiles
- [ ] Implement Profile data model and ProfileManager
- [ ] Wire profile-scoped storage and status
- [ ] Add CLI flags and subcommand
- [ ] Implement auto-migration to profile directories
- [ ] Unit tests for profile model
- [ ] Code review

### Epics 8–10: Testing and Documentation
- [ ] Integration tests complete
- [ ] Manual E2E verification
- [ ] CLAUDE.md, e2e-testing.md, BACKLOG.md updated
- [ ] Final review
- [ ] Merge to main
- [ ] Archive design/plan docs

---

## Epic Status Tracking

| Epic | Status | Start Date | End Date | Notes |
|------|--------|------------|----------|-------|
| Epic 1 (Daemon Auth) | Not Started | - | - | |
| Epic 2 (Status File) | Not Started | - | - | |
| Epic 3 (Credential Storage) | Not Started | - | - | |
| Epic 4 (Profile Model) | Not Started | - | - | Blocked by Epics 2–3 |
| Epic 5 (Profile Storage) | Not Started | - | - | Blocked by Epics 2–4 |
| Epic 6 (CLI Changes) | Not Started | - | - | Blocked by Epics 4–5 |
| Epic 7 (Migration) | Not Started | - | - | Blocked by Epics 2–4 |
| Epic 8 (Integration Tests) | Not Started | - | - | Blocked by Epics 1–7 |
| Epic 9 (Manual E2E Tests) | Not Started | - | - | Blocked by Epics 1–7 |
| Epic 10 (Documentation) | Not Started | - | - | Blocked by Epics 1–7 |

---

## Open Questions

| # | Question | Status | Blocker For |
|---|----------|--------|-------------|
| 1 | Should `--use-keyring` also apply to `logout` command? | Open | Epic 3 |
| 2 | Should profile deletion require the instance to be stopped first? | Open | Epic 6 |
| 3 | Should migration log to stderr or to a log file? | Deferred | Epic 7 |

---

## References

- [DAEMON-AUTH.design.md](./DAEMON-AUTH.design.md)
- [HOME-DIR-STATUS.design.md](./HOME-DIR-STATUS.design.md)
- [HOME-DIR-TOKEN.design.md](./HOME-DIR-TOKEN.design.md)
- [MULTI-INSTANCE-PROFILES.design.md](./MULTI-INSTANCE-PROFILES.design.md)
- [BACKLOG.md](./BACKLOG.md)

---

## Overlap Eliminated

The following redundancies from the individual plans are resolved:

| Redundancy | Individual Plans | Consolidated Approach |
|------------|-----------------|----------------------|
| Create `~/.copilot-adapter/` directory | HOME-DIR-STATUS + HOME-DIR-TOKEN both create it | Created once in `get_base_dir()` (Epic 2) |
| `create_storage()` API change | HOME-DIR-TOKEN adds `use_keyring` param; MULTI-INSTANCE-PROFILES adds `for_profile()` | Build `create_storage_with_path(path, use_keyring)` once (Epic 3), add `for_profile()` wrapper (Epic 5) |
| Status functions API | HOME-DIR-STATUS creates fixed-path functions; MULTI-INSTANCE-PROFILES refactors to accept paths | Build parameterized `*_to()`/`*_from()` API from day one (Epic 2) |
| `remove_all_status_files()` | HOME-DIR-STATUS creates it; MULTI-INSTANCE-PROFILES changes to profile-scoped | Build cleanup that handles both legacy and profile paths |
| Documentation updates | Each plan has "Update CLAUDE.md", "Update BACKLOG.md", "Update e2e-testing.md" | Single documentation pass at the end (Epic 10) |
| Testing epics | 4 separate testing epics | Unit tests per epic, integration + E2E once at end |

---

## Notes

### Development Notes
- [Notes added during implementation]

### Review Notes
- [Code review feedback]

### Testing Notes
- [Test failures and fixes]
- [Edge cases discovered]
