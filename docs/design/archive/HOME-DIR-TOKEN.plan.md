# Home Directory Token Storage — Implementation Plan

**Status:** Not Started
**Date:** 2026-04-01
**Based on:** [HOME-DIR-TOKEN.design.md](./HOME-DIR-TOKEN.design.md)
**Prerequisite:** HOME-DIR-STATUS (shared `~/.copilot-adapter/` directory)
**Estimated Time:** 1-2 days

---

## Executive Summary

This plan flips the credential storage priority from keyring-first to file-first, unifies the storage path to `~/.copilot-adapter/credentials.json`, and adds a `--use-keyring` CLI flag for opt-in keyring usage. Includes migration from old file paths.

---

## Implementation Plan

### Epic 1: Unify Storage Directory

**Status:** Not Started

#### Task 1.1: Update FileStorage path resolution

**File:** `src/storage/file.rs` (MODIFIED)

Change `get_credentials_path()` to use `~/.copilot-adapter/credentials.json` as primary, with fallback to existing platform-specific paths. Add `get_legacy_credentials_path()` for the old paths.

#### Task 1.2: Add migration logic

**File:** `src/storage/file.rs` (MODIFIED)

In `FileStorage::new()` or on first `get_github_token()`, check if credentials exist at old path but not new path. If so, copy to new location.

```rust
fn migrate_if_needed(&self) {
    let new_path = get_credentials_path();
    if new_path.exists() { return; }
    let old_path = get_legacy_credentials_path();
    if old_path.exists() && old_path != new_path {
        let _ = std::fs::create_dir_all(new_path.parent().unwrap());
        let _ = std::fs::copy(&old_path, &new_path);
    }
}
```

**Acceptance Criteria:**
- [ ] Default path is `~/.copilot-adapter/credentials.json`
- [ ] Migration from old path works
- [ ] Fallback to legacy path when home not writable

---

### Epic 2: Flip Storage Priority

**Status:** Not Started

#### Task 2.1: Add use_keyring parameter

**File:** `src/storage/mod.rs` (MODIFIED)

**Before:**
```rust
pub fn create_storage() -> Box<dyn TokenStorage + Send + Sync> {
    match keyring::KeyringStorage::new() {
        Ok(ks) => {
            if ks.verify_available().unwrap_or(false) {
                return Box::new(ks);
            }
        }
        Err(_) => {}
    }
    Box::new(file::FileStorage::new())
}
```

**After:**
```rust
pub fn create_storage(use_keyring: bool) -> Box<dyn TokenStorage + Send + Sync> {
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
    Box::new(file::FileStorage::new())
}
```

**Acceptance Criteria:**
- [ ] `create_storage(false)` returns FileStorage
- [ ] `create_storage(true)` tries keyring, falls back to file

---

### Epic 3: CLI Flag

**Status:** Not Started

#### Task 3.1: Add --use-keyring to CLI

**File:** `src/cli.rs` (MODIFIED)

```rust
Start {
    // ... existing flags ...
    /// Use OS keyring for credential storage instead of encrypted file
    #[arg(long)]
    use_keyring: bool,
}
```

Also add to `Auth` command.

#### Task 3.2: Pass flag through main.rs

**File:** `src/main.rs` (MODIFIED)

Update all `storage::create_storage()` calls to pass `use_keyring`:
```rust
let store = storage::create_storage(use_keyring);
```

**Acceptance Criteria:**
- [ ] `--use-keyring` flag on `start` and `auth` commands
- [ ] Flag passed to `create_storage()`

---

### Epic 4: Testing

**Status:** Not Started

- Unit tests: path resolution, migration logic, create_storage with both flags
- Integration tests: auth/logout cycle with file storage
- Manual E2E: verify file at `~/.copilot-adapter/credentials.json`

---

### Epic 5: Documentation

**Status:** Not Started

- Update CLAUDE.md: note about file storage as default, `--use-keyring` option
- Update BACKLOG.md: move item to Done
- Update CLI help text

---

## File Changes Summary

| File | Change | Description |
|------|--------|-------------|
| `src/storage/file.rs` | Modified | New path, migration logic |
| `src/storage/mod.rs` | Modified | `create_storage(use_keyring: bool)` |
| `src/cli.rs` | Modified | Add `--use-keyring` flag |
| `src/main.rs` | Modified | Pass use_keyring to create_storage() |
| `CLAUDE.md` | Modified | Update storage notes |
| `docs/design/BACKLOG.md` | Modified | Move to Done |

---

## Risk Assessment

| Risk | Impact | Probability | Mitigation |
|------|--------|-------------|------------|
| Existing keyring users lose credentials | Medium | Medium | Keyring fallback if file empty; migration guide in docs |
| Home dir not writable | Low | Low | Fallback to legacy path |
| Migration race | Low | Very Low | Idempotent copy |

---

## Success Criteria

1. Default storage at `~/.copilot-adapter/credentials.json`
2. `--use-keyring` enables OS keyring
3. Migration from old paths works
4. All tests pass

---

## References

- [HOME-DIR-TOKEN.design.md](./HOME-DIR-TOKEN.design.md)
- [HOME-DIR-STATUS.design.md](./HOME-DIR-STATUS.design.md)
- [BACKLOG.md](./BACKLOG.md)
