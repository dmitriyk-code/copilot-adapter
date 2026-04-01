# Multi-Instance Profiles — Implementation Plan

**Status:** Not Started
**Date:** 2026-04-01
**Based on:** [MULTI-INSTANCE-PROFILES.design.md](./MULTI-INSTANCE-PROFILES.design.md)
**Prerequisite:** HOME-DIR-STATUS, HOME-DIR-TOKEN
**Estimated Time:** 3-5 days

---

## Executive Summary

This plan implements the multi-instance profile system, enabling concurrent copilot-adapter instances with independent credentials and ports. The implementation spans 7 epics: profile data model, profile-scoped storage, CLI changes, main entry updates, migration, testing, and documentation. Total estimated effort: 3-5 days.

---

## Background

### Current State
- Single instance: one PID file, one credential store, one port
- No profile concept; all state in flat files

### Target State
- Profile-scoped directories: `~/.copilot-adapter/profiles/<name>/`
- Each profile: own `status.json` + `credentials.json`
- CLI `--profile` / `-P` flag on all commands
- Default profile "default" preserves backward compatibility

---

## Implementation Plan

### Epic 1: Profile Data Model (Day 1)

**Status:** Not Started
**Objective:** Create Profile struct, validation, and ProfileManager.

#### Task 1.1: Create profile types

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

#### Task 1.2: Create ProfileManager

**File:** `src/profile/mod.rs` (NEW)

Implement `new()`, `get()`, `list()`, `create()`, `delete()`, `find_by_port()`, `check_port_conflict()`.

#### Task 1.3: Register module

**File:** `src/lib.rs` (MODIFIED) — Add `pub mod profile;`

**Acceptance Criteria:**
- [ ] Profile struct with status_path() and credentials_path()
- [ ] Name validation: alphanumeric + dash + underscore, 1-64 chars
- [ ] ProfileManager CRUD operations
- [ ] Port conflict detection across profiles

---

### Epic 2: Profile-Scoped Storage (Day 1-2)

**Status:** Not Started
**Objective:** Make storage and status accept profile-specific paths.

#### Task 2.1: Parameterize FileStorage

**File:** `src/storage/file.rs` (MODIFIED)

Add constructor that accepts a custom credentials path:
```rust
impl FileStorage {
    pub fn new() -> Self { /* default path */ }
    pub fn with_path(path: PathBuf) -> Self { /* custom path */ }
}
```

#### Task 2.2: Parameterize create_storage

**File:** `src/storage/mod.rs` (MODIFIED)

```rust
pub fn create_storage_for_profile(profile: &Profile, use_keyring: bool) -> Box<dyn TokenStorage + Send + Sync> {
    if use_keyring { /* try keyring */ }
    Box::new(file::FileStorage::with_path(profile.credentials_path()))
}
```

#### Task 2.3: Parameterize status functions

**File:** `src/daemon/status.rs` (MODIFIED)

Add profile-aware variants:
```rust
pub fn write_status_to(path: &Path, port: u16) -> Result<()> { /* ... */ }
pub fn read_status_from(path: &Path) -> Option<StatusFile> { /* ... */ }
pub fn remove_status_from(path: &Path) { /* ... */ }
```

---

### Epic 3: CLI Changes (Day 2)

**Status:** Not Started
**Objective:** Add --profile, --all flags and profiles subcommand.

#### Task 3.1: Add profile flags

**File:** `src/cli.rs` (MODIFIED)

- Add `--profile` / `-P` with default "default" to Start, Stop, Status, Auth, Logout
- Add `--all` flag to Stop and Status
- Add `Profiles` subcommand with `List`, `Create { name }`, `Delete { name }`

---

### Epic 4: Main Entry Updates (Day 2-3)

**Status:** Not Started
**Objective:** Wire profile resolution into all command handlers.

#### Task 4.1: Profile resolution at startup

**File:** `src/main.rs` (MODIFIED)

```rust
let pm = ProfileManager::new();
let profile = pm.get(&profile_name)?;
```

#### Task 4.2: Profile-scoped start

Check port conflicts, use profile-scoped storage and status.

#### Task 4.3: Profile-scoped stop/status

`--all` iterates all profiles. Single profile operates on named profile.

#### Task 4.4: Profiles subcommand handler

Handle `profiles list`, `profiles create <name>`, `profiles delete <name>`.

---

### Epic 5: Migration and Backward Compatibility (Day 3)

**Status:** Not Started
**Objective:** Migrate existing single-instance data to default profile.

#### Task 5.1: Auto-migration on first run

If `~/.copilot-adapter/profiles/` doesn't exist but `~/.copilot-adapter/status.json` or `~/.copilot-adapter/credentials.json` does:
1. Create `~/.copilot-adapter/profiles/default/`
2. Move status.json and credentials.json into it
3. Log the migration

#### Task 5.2: Legacy temp dir migration

Check temp dir PID file and migrate to default profile status.

---

### Epic 6: Testing (Day 3-4)

**Status:** Not Started

#### Task 6.1: Unit tests

- Profile name validation (valid, invalid, edge cases, empty, 65 chars)
- ProfileManager CRUD
- Port conflict detection
- Default profile behavior

#### Task 6.2: Integration tests

- Full lifecycle: create → auth → start → status → stop → delete
- Two profiles simultaneously
- Port conflict rejection
- Migration from single-instance to default profile
- `--all` operations

#### Task 6.3: Manual E2E tests

- Create work profile, auth, start on different port
- `status --all` shows both instances
- `stop --all` stops all
- Delete work profile

---

### Epic 7: Documentation (Day 4)

**Status:** Not Started

- CLAUDE.md: Add profile/ to project structure, profile notes, updated CLI commands table
- docs/development/e2e-testing.md: Multi-instance test procedures
- BACKLOG.md: Move item to Done
- CLI help text: `--profile` / `-P` descriptions

---

## File Changes Summary

| File | Change | Description |
|------|--------|-------------|
| `src/profile/mod.rs` | **New file** | ProfileManager |
| `src/profile/types.rs` | **New file** | Profile struct, validation |
| `src/lib.rs` | Modified | Add `pub mod profile` |
| `src/cli.rs` | Modified | --profile, --all, Profiles subcommand |
| `src/main.rs` | Modified | Profile resolution, all command handlers |
| `src/storage/mod.rs` | Modified | Profile-aware create_storage |
| `src/storage/file.rs` | Modified | Custom path constructor |
| `src/daemon/status.rs` | Modified | Profile-aware status functions |
| `src/daemon/mod.rs` | Modified | Profile-aware public API |
| `tests/unit/profile_tests.rs` | **New file** | Profile unit tests |
| `tests/integration/profile_tests.rs` | **New file** | Profile integration tests |
| `CLAUDE.md` | Modified | Profile documentation |
| `docs/design/BACKLOG.md` | Modified | Move to Done |

---

## Dependencies

### New Dependencies
None — uses existing `serde`, `serde_json`, `dirs`, `anyhow`.

### Sequencing
1. HOME-DIR-STATUS must be complete (provides StatusFile and get_status_dir)
2. HOME-DIR-TOKEN must be complete (provides file-based credential storage)
3. Then this feature builds on both

---

## Risk Assessment

| Risk | Impact | Probability | Mitigation |
|------|--------|-------------|------------|
| Breaking backward compat | High | Low | Default profile = existing behavior; comprehensive testing |
| Port conflicts | Medium | Medium | Explicit check at startup; clear error message |
| Migration data loss | High | Low | Copy (not move) during migration; verify before delete |
| Complex CLI changes | Medium | Medium | Incremental implementation; test each flag |
| Profile directory corruption | Low | Low | Validate name, create_dir_all, handle IO errors |

---

## Timeline

| Epic | Days | Dependencies |
|------|------|-------------|
| Epic 1: Profile data model | 0.5 | None |
| Epic 2: Profile-scoped storage | 0.5 | Epic 1 |
| Epic 3: CLI changes | 0.5 | Epic 1 |
| Epic 4: Main entry updates | 1.0 | Epics 1-3 |
| Epic 5: Migration | 0.5 | Epic 4 |
| Epic 6: Testing | 1.0 | Epic 5 |
| Epic 7: Documentation | 0.5 | Epic 6 |
| **Total** | **3-5** | |

---

## Success Criteria

1. `copilot-adapter start` works identically to today (default profile)
2. `copilot-adapter start -P work -p 8080` starts a second instance
3. `copilot-adapter status --all` shows all running instances
4. `copilot-adapter stop --all` stops all instances
5. Port conflict detection prevents duplicate binds
6. Migration from single-instance is seamless
7. All tests pass: `cargo test`

---

## References

- [MULTI-INSTANCE-PROFILES.design.md](./MULTI-INSTANCE-PROFILES.design.md)
- [HOME-DIR-STATUS.design.md](./HOME-DIR-STATUS.design.md)
- [HOME-DIR-TOKEN.design.md](./HOME-DIR-TOKEN.design.md)
- [BACKLOG.md](./BACKLOG.md)
