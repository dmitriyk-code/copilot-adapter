# Home Directory Token Storage — Design Document

**Status:** Proposed
**Date:** 2026-04-01
**Severity:** Low
**Related:** [HOME-DIR-STATUS.design.md](./HOME-DIR-STATUS.design.md)

---

## Executive Summary

The copilot-adapter uses a three-tier credential storage system: OS keyring (primary), encrypted file (fallback), and Windows Credential Manager (Windows-specific). The keyring approach is unreliable in certain environments (corporate policies, containers, headless Linux). This design proposes flipping the priority to make encrypted file storage the primary mechanism, stored under `~/.copilot-adapter/credentials.json` (unified with the status file directory). The OS keyring becomes an opt-in alternative via `--use-keyring`.

---

## Context / Background

### Current State

**Storage trait** (`src/storage/mod.rs`):
```rust
pub trait TokenStorage: Send + Sync {
    fn store_github_token(&self, token: &str) -> Result<()>;
    fn get_github_token(&self) -> Result<String>;
    fn delete_github_token(&self) -> Result<()>;
}
```

**Three-tier system:**
1. **KeyringStorage** (`keyring.rs`): Uses `keyring` crate v3.6. Service "copilot-adapter", key "github_token". `verify_available()` does round-trip probe test. On Windows, uses custom `LocalMachineCredential` with `CRED_PERSIST_LOCAL_MACHINE`.
2. **FileStorage** (`file.rs`): XOR obfuscation with username-mixed key. Platform-specific paths: `%APPDATA%\copilot-adapter\credentials.json` (Windows), `~/Library/Application Support/copilot-adapter/credentials.json` (macOS), `~/.config/copilot-adapter/credentials.json` (Linux).
3. **Selection** (`mod.rs`): `create_storage()` tries keyring first, falls back to file.

### Target State

- **Primary:** FileStorage at `~/.copilot-adapter/credentials.json`
- **Optional:** KeyringStorage via `--use-keyring` CLI flag
- **Unified directory:** Same `~/.copilot-adapter/` as status.json
- **Migration:** Auto-migrate from old file paths on first access

---

## Problem Statement

**Observed behavior:**
- Keyring fails silently in some environments (Docker, WSL, corporate lockdown)
- Users unaware which storage backend is active
- File fallback uses different directory than planned status file

**Expected behavior:**
- Predictable file-based storage as default
- Single `~/.copilot-adapter/` directory for all adapter state
- Keyring available as opt-in for users who prefer it

---

## Goals and Non-Goals

### Goals

| ID | Goal | Success Criteria |
|----|------|------------------|
| G1 | File storage as primary | `create_storage()` returns FileStorage by default |
| G2 | Unified directory `~/.copilot-adapter/` | Credentials stored alongside status.json |
| G3 | `--use-keyring` opt-in flag | Keyring available when explicitly requested |
| G4 | Migrate from old file paths | Existing credentials auto-discovered |
| G5 | Backward compatible | Users with keyring credentials not disrupted |

### Non-Goals

| ID | Non-Goal | Rationale |
|----|----------|-----------|
| NG1 | Upgrade encryption (AES-GCM) | XOR adequate for local proxy token; avoids crypto deps |
| NG2 | Remove keyring support entirely | Some users prefer OS-level credential management |
| NG3 | Encrypt status.json | Status data (PID, port) is not sensitive |

---

## Research / Analysis

### Encryption Options

| Option | Security | Dependencies | Complexity |
|--------|----------|-------------|------------|
| XOR obfuscation (current) | Low — prevents casual reading | None | Low |
| AES-256-GCM | High — cryptographically secure | `aes-gcm` crate (~50KB) | Medium |
| OS keyring | High — OS-managed | `keyring` crate (already present) | Low |

**Recommendation:** Keep XOR obfuscation. The token stored is a GitHub OAuth token for a localhost proxy. The threat model is "prevent casual reading by other users on shared machine." XOR is sufficient for this, and avoids adding crypto dependencies that increase binary size and audit surface.

The file already has Unix permissions `0o600` (owner read/write only), which provides the primary access control.

---

## Proposed Design

### Modified create_storage()

```rust
// src/storage/mod.rs

/// Create the storage backend.
/// Default: encrypted file storage at ~/.copilot-adapter/
/// With use_keyring=true: OS keyring (falls back to file on failure)
pub fn create_storage(use_keyring: bool) -> Box<dyn TokenStorage + Send + Sync> {
    if use_keyring {
        // Try keyring first when explicitly requested
        match keyring::KeyringStorage::new() {
            Ok(ks) => {
                if ks.verify_available().unwrap_or(false) {
                    tracing::info!("Using OS keyring for credential storage");
                    return Box::new(ks);
                }
                tracing::warn!("OS keyring not available, falling back to file storage");
            }
            Err(e) => {
                tracing::warn!("Failed to initialize keyring: {e}, using file storage");
            }
        }
    }
    // Default: file storage
    tracing::info!("Using file-based credential storage");
    Box::new(file::FileStorage::new())
}
```

### Modified FileStorage path

```rust
// src/storage/file.rs

fn get_credentials_path() -> PathBuf {
    // Primary: ~/.copilot-adapter/credentials.json
    if let Some(home) = dirs::home_dir() {
        let dir = home.join(".copilot-adapter");
        if std::fs::create_dir_all(&dir).is_ok() {
            return dir.join("credentials.json");
        }
    }
    // Fallback: platform-specific config dir (existing behavior)
    get_legacy_credentials_path()
}

fn get_legacy_credentials_path() -> PathBuf {
    // Existing platform-specific logic
    // %APPDATA%/copilot-adapter/ (Windows)
    // ~/Library/Application Support/copilot-adapter/ (macOS)
    // ~/.config/copilot-adapter/ (Linux)
}
```

### Migration logic

```rust
fn migrate_if_needed(new_path: &Path) -> Option<()> {
    if new_path.exists() { return Some(()); } // Already migrated
    let old_path = get_legacy_credentials_path();
    if old_path.exists() && old_path != new_path {
        // Copy old file to new location
        std::fs::copy(&old_path, new_path).ok()?;
        tracing::info!("Migrated credentials from {} to {}", old_path.display(), new_path.display());
    }
    Some(())
}
```

---

## Design Decisions

| Decision | Rationale |
|----------|-----------|
| File storage as primary | More reliable across environments than keyring |
| Keep XOR encryption | Adequate for threat model; no new dependencies |
| `--use-keyring` opt-in | Users who want OS keyring can still use it |
| Same `~/.copilot-adapter/` directory | Unified state location with status.json |
| Migration by copy (not move) | Old path still works if user downgrades |
| `create_storage(use_keyring: bool)` parameter | Clean API; caller decides based on CLI flag |

---

## File Changes Summary

| File | Change | Description |
|------|--------|-------------|
| `src/storage/mod.rs` | Modified | Add `use_keyring` parameter to `create_storage()` |
| `src/storage/file.rs` | Modified | Change default path to `~/.copilot-adapter/`, add migration |
| `src/cli.rs` | Modified | Add `--use-keyring` flag to `Start` and `Auth` commands |
| `src/main.rs` | Modified | Pass `use_keyring` to `create_storage()` calls |

---

## Testing Strategy

### Unit Tests
1. New path resolution returns `~/.copilot-adapter/credentials.json`
2. Migration copies from old path to new path
3. Fallback to legacy path when home not writable
4. `create_storage(false)` returns FileStorage
5. `create_storage(true)` returns KeyringStorage when available

### Integration Tests
1. Auth → store token → restart → token loaded from new path
2. Migration: place file at old path, verify loaded from new path after migration

### Manual E2E Tests
1. `copilot-adapter auth` → verify `~/.copilot-adapter/credentials.json` created
2. `copilot-adapter auth --use-keyring` → verify keyring used
3. `copilot-adapter logout` → verify file removed

---

## Risk Assessment

| Risk | Impact | Probability | Mitigation |
|------|--------|-------------|------------|
| Existing keyring users lose credentials | Medium | Medium | First `get_github_token()` check: try keyring if file missing |
| XOR encryption deemed insufficient | Low | Low | Document in security notes; recommend `--use-keyring` for high-security |
| Migration race condition | Low | Very Low | Idempotent copy operation |
| Home dir not writable | Low | Low | Fallback to legacy platform-specific path |

---

## Success Criteria

1. Default storage at `~/.copilot-adapter/credentials.json`
2. `--use-keyring` flag enables OS keyring
3. Existing credentials migrated transparently
4. All tests pass

---

## References

- `src/storage/mod.rs` — Current storage selection logic
- `src/storage/file.rs` — Encrypted file storage implementation
- `src/storage/keyring.rs` — OS keyring storage implementation
- [HOME-DIR-STATUS.design.md](./HOME-DIR-STATUS.design.md) — Shared directory design
