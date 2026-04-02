# Native Credential Storage — Design Document

**Status:** Proposed
**Date:** 2026-04-02
**Severity:** Medium
**Related:** [HOME-DIR-TOKEN.design.md](./HOME-DIR-TOKEN.design.md), [CONSOLIDATED.plan.md](./CONSOLIDATED.plan.md)

---

## Executive Summary

The copilot-adapter currently uses XOR obfuscation for file-based credential storage (`credentials.json`) with an optional `--use-keyring` flag for OS keyring access. XOR is not cryptographic — it is trivially reversible if the OS username is known. The `--use-keyring` flag creates a confusing two-tier system where users must explicitly opt in to real security.

This design replaces the entire storage system with **always-on platform-native encryption** that requires no user flags:

- **Windows:** DPAPI (`CryptProtectData` / `CryptUnprotectData`) — encrypts with the current user's Windows credentials
- **macOS:** Keychain via the existing `keyring` crate
- **Linux:** Secret Service via the existing `keyring` crate
- **No fallback:** If no native encryption is available (e.g., headless Linux without Secret Service), the adapter **refuses to store credentials** and shows a clear error. We do not store secrets in an insecure way.

The credential file is renamed from `credentials.json` to `github-copilot.json` and uses a human-readable JSON format with the encrypted token stored as a base64 value.

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

**Two-tier system with opt-in flag:**
1. **FileStorage** (`file.rs`): XOR obfuscation with a key derived from a hardcoded string mixed with the OS username. Stores opaque binary to `~/.copilot-adapter/credentials.json`.
2. **KeyringStorage** (`keyring.rs`): Uses `keyring` crate v3.6 with `verify_available()` probe. On Windows, uses custom `LocalMachineCredential` with `CRED_PERSIST_LOCAL_MACHINE` (Windows Credential Manager).
3. **Selection** (`mod.rs`): `create_storage_with_path(path, use_keyring)` — if `use_keyring` is true, tries keyring first, falls back to file. Otherwise uses file directly.

**Problems:**
- XOR obfuscation is trivially reversible — the "key" is deterministic from a fixed string and the username
- Users must pass `--use-keyring` to get real encryption — most don't know this
- Keyring entries are shared across profiles (documented limitation)
- Logout must clear both file and keyring backends (two-pass logic in `main.rs`)
- The file extension `.json` is misleading — the file contains opaque binary, not JSON

### Target State

- **Single `NativeStorage` backend** — automatically selects the best OS mechanism
- **No `--use-keyring` flag** — security is always on by default
- **Human-readable JSON file** (`github-copilot.json`) — secret value is encrypted+base64 or stored in keyring
- **Profile-scoped keyring entries** — each profile gets its own keyring key
- **Simple logout** — single `delete_github_token()` call handles everything

---

## Problem Statement

**Observed behavior:**
- File-based credentials use reversible XOR obfuscation that provides no real security
- Users must explicitly opt in to OS keyring via `--use-keyring` flag
- The credential file is opaque binary despite having a `.json` extension
- Keyring entries are shared across profiles, causing cross-contamination

**Expected behavior:**
- Credentials are encrypted with platform-native mechanisms by default
- No user flags needed — security is automatic
- Credential file is human-readable JSON (inspectable, debuggable)
- Each profile has isolated credential storage

**Impact:**
- All users on all platforms benefit from stronger default security
- Simpler CLI with fewer flags to understand
- Cleaner codebase with one storage backend instead of two

---

## Goals and Non-Goals

### Goals

| ID | Goal | Success Criteria |
|----|------|------------------|
| G1 | Platform-native encryption by default | Windows uses DPAPI, macOS uses Keychain, Linux uses Secret Service — no flags needed |
| G2 | Remove `--use-keyring` flag | CLI simplified; `Start` and `Auth` commands lose the flag |
| G3 | Human-readable credential file | `github-copilot.json` is valid JSON with `version`, `storage`, and optional `github_token` fields |
| G4 | Profile-scoped keyring entries | Each profile stores credentials independently in the OS keyring |
| G5 | Transparent migration with security priority | Old `credentials.json` (XOR format) auto-deleted on first access; migration best-effort; **priority is removing insecure storage**, not preserving credentials; re-authentication if migration fails is acceptable |
| G6 | Refuse insecure storage | If no native encryption is available, refuse to save credentials and show a clear error guiding the user |

### Non-Goals

| ID | Non-Goal | Rationale |
|----|----------|-----------|
| NG1 | Add third-party encryption crate (AES-GCM, etc.) | DPAPI is OS-native on Windows; keyring handles macOS/Linux; no need for extra deps |
| NG2 | Encrypt non-secret data (status.json, profiles) | Only the GitHub OAuth token is sensitive |
| NG3 | Support custom encryption keys or passphrases | DPAPI and keyring derive keys from OS user credentials — no user key management |
| NG4 | Remove `keyring` crate entirely | Still needed for macOS Keychain and Linux Secret Service |

---

## Research / Analysis

### Windows DPAPI

**What it is:** Data Protection API, built into Windows since Windows 2000. Provides `CryptProtectData` and `CryptUnprotectData` functions that encrypt/decrypt arbitrary byte blobs using the current user's credentials.

**Key properties:**
- Encryption key derived from the user's Windows login password + machine-specific entropy
- No key management needed — just call the function with plaintext, get back ciphertext
- Only the same user on the same machine can decrypt
- Uses AES-256 internally (OS-managed)
- Built-in integrity checking

**Availability:** Always available on all supported Windows versions. No probing needed.

**Failure mode:** If the Windows user profile is reset (not changed, but *reset* by an admin), old DPAPI-encrypted data becomes unreadable. This is acceptable — the adapter prompts re-authentication, same as the current XOR behavior when the username changes.

**FFI via `windows-sys`:** The `Win32_Security_Cryptography` feature exposes:
- `CryptProtectData(pDataIn, szDataDescr, pOptionalEntropy, pvReserved, pPromptStruct, dwFlags, pDataOut) -> BOOL`
- `CryptUnprotectData(pDataIn, pszDataDescr, pOptionalEntropy, pvReserved, pPromptStruct, dwFlags, pDataOut) -> BOOL`
- `CRYPT_INTEGER_BLOB { cbData: u32, pbData: *mut u8 }` — input/output data structure
- Memory cleanup via `LocalFree()` from `Win32::Foundation`

### macOS Keychain / Linux Secret Service

Already implemented via the `keyring` crate. The existing `KeyringStorage` code works well — it just needs to be merged into the unified `NativeStorage` and made the default instead of opt-in.

### No-Storage Fallback Strategy

On headless Linux (no D-Bus, no Secret Service), the keyring probe fails. Rather than falling back to plaintext or XOR (which both provide a false sense of security), the new design **refuses to store credentials** when no native encryption is available:

- `store_github_token()` returns an error with a clear message:
  *"No secure credential storage available. On Linux, install and start a Secret Service provider (e.g., GNOME Keyring, KDE Wallet, or `pass`). Then re-run `copilot-adapter auth`."*
- `get_github_token()` returns a "no token stored" error as usual
- The adapter can still start with `--skip-auth` for environments that manage tokens externally

This is the secure default — we never store secrets without proper encryption. Users in environments without a keyring must install one or provide tokens via other means.

---

## Proposed Design

### Architecture

```
create_storage_for_profile(profile)
    |
    v
NativeStorage::new(file_path)
    |
    +-- detect_method()
    |       |
    |       +-- Windows --> StorageMethod::Dpapi
    |       +-- macOS   --> probe keyring --> Keyring or Unavailable
    |       +-- Linux   --> probe keyring --> Keyring or Unavailable
    |
    +-- store_github_token(token)
    |       |
    |       +-- Dpapi:       encrypt(token) --> base64 --> write JSON file
    |       +-- Keyring:     keyring.set_password(token) --> write sentinel JSON
    |       +-- Unavailable: return Err("no secure storage available")
    |
    +-- get_github_token()
    |       |
    |       +-- read JSON file --> check "storage" field
    |       +-- Dpapi:       base64 decode --> decrypt --> return
    |       +-- Keyring:     keyring.get_password() --> return
    |       +-- No file:     return Err("no token stored")
    |
    +-- delete_github_token()
            |
            +-- Dpapi:       delete JSON file
            +-- Keyring:     keyring.delete_credential() + delete JSON file
            +-- No file:     no-op
```

### File Format (`github-copilot.json`)

**Windows (DPAPI-encrypted):**
```json
{
  "version": 2,
  "storage": "dpapi",
  "github_token": "AQAAANCMnd8BFdERjHoAwE/Cl+sBAAAA..."
}
```

**macOS/Linux (keyring-backed, sentinel file):**
```json
{
  "version": 2,
  "storage": "keyring"
}
```
Token is stored in OS keyring under service `copilot-adapter`, user `{profile_name}:github_token`.

**No secure storage available:** No file is written. `store_github_token()` returns an error.

### Struct Definitions

```rust
/// On-disk credential file format (v2).
#[derive(Serialize, Deserialize)]
struct CredentialFile {
    /// Schema version. Always 2 for the new format.
    version: u32,
    /// How the token is protected: "dpapi" or "keyring".
    storage: String,
    /// The token value. For "dpapi": base64-encoded encrypted blob.
    /// For "keyring": absent (token is in OS keyring).
    #[serde(skip_serializing_if = "Option::is_none")]
    github_token: Option<String>,
}

/// Which protection mechanism is in use.
#[derive(Debug, Clone, Copy, PartialEq)]
enum StorageMethod {
    /// Windows DPAPI — encrypted blob in JSON file.
    Dpapi,
    /// OS keyring (macOS Keychain / Linux Secret Service).
    Keyring,
    /// No secure storage available — store/get will return errors.
    Unavailable,
}
```

### `get_github_token()` Retrieval Strategy

The retrieval reads the file's `"storage"` field to decide how to retrieve — **not** the currently-detected method. This prevents silent data loss:

1. User stores token when keyring is available (`storage: "keyring"`)
2. Later, keyring daemon crashes
3. `detect_method()` now returns `Unavailable`
4. But the token is in the keyring, not the file
5. Reading by the file's `storage` field surfaces the keyring error clearly, prompting re-authentication

### Profile-Scoped Keyring Entries

Current: All profiles share `copilot-adapter` / `github_token` in the keyring.

New: Keyring user key is `"{profile_name}:github_token"`, derived from the profile directory name. Example:
- Profile `default` → keyring user `default:github_token`
- Profile `work` → keyring user `work:github_token`

### DPAPI Module (`src/storage/dpapi.rs`)

```rust
#[cfg(target_os = "windows")]
pub fn encrypt(plaintext: &[u8]) -> Result<Vec<u8>> {
    unsafe {
        let mut input = CRYPT_INTEGER_BLOB {
            cbData: plaintext.len() as u32,
            pbData: plaintext.as_ptr() as *mut u8,
        };
        let mut output = CRYPT_INTEGER_BLOB {
            cbData: 0,
            pbData: std::ptr::null_mut(),
        };
        let result = CryptProtectData(
            &mut input,
            std::ptr::null(),     // no description
            std::ptr::null_mut(), // no entropy
            std::ptr::null_mut(), // reserved
            std::ptr::null_mut(), // no prompt
            0,                    // current-user scope
            &mut output,
        );
        if result == 0 {
            return Err(anyhow!("CryptProtectData failed: {}", std::io::Error::last_os_error()));
        }
        let encrypted = std::slice::from_raw_parts(output.pbData, output.cbData as usize).to_vec();
        LocalFree(output.pbData as *mut _);
        Ok(encrypted)
    }
}

#[cfg(target_os = "windows")]
pub fn decrypt(encrypted: &[u8]) -> Result<Vec<u8>> {
    // Symmetric structure using CryptUnprotectData
    // ...
}
```

### Migration

**Trigger:** When `NativeStorage::new()` is called and `github-copilot.json` does NOT exist.

**Sources checked (in order):**
1. `credentials.json` in the same directory (XOR-obfuscated format)
2. Old keyring entry (`copilot-adapter` / `github_token`) on macOS/Linux

**Process:**
1. Read old token from source (best-effort)
2. Store via `NativeStorage` (creates `github-copilot.json`) if read succeeds
3. **Delete** old `credentials.json` (removes insecure XOR format)
4. Delete old keyring entry if migrated from there

**Important**: If migration fails (corrupted file, username changed, etc.), the old file is still deleted and the user is prompted to re-authenticate. The goal is to **stop storing tokens in XOR format**, not to preserve backward compatibility at all costs.

### Backward Compatibility — Detailed Specification

#### Migration Trigger Conditions

The migration logic is invoked in `NativeStorage::new()` and checks the following conditions in order:

1. **Primary check**: `github-copilot.json` does NOT exist in the profile directory
2. **Secondary check**: `credentials.json` (old XOR format) DOES exist in the same directory
3. **Edge case**: Both files exist → remove old `credentials.json`, use new format (no migration)

#### Old XOR Format Detection

The old format is detected by:
- **Filename**: `credentials.json` (same name, different content format)
- **Content**: Opaque binary data (XOR-obfuscated JSON, despite `.json` extension)
- **Structure when decrypted**: `{"github_token": "ghp_..."}`

**Key characteristics:**
- No `version` field (implicit version 1)
- Binary file, not human-readable
- Key derived from fixed string + OS username
- Symmetric XOR transform for encryption/decryption

#### Legacy Module (`src/storage/legacy.rs`)

Extract XOR read functions from current `src/storage/file.rs` for migration use:

```rust
/// Read a GitHub token from an old XOR-obfuscated credentials.json file.
///
/// Returns:
/// - `Ok(token)` if the token was successfully read
/// - `Err(...)` with descriptive error if:
///   - File doesn't exist
///   - File is corrupted
///   - OS username has changed (XOR key mismatch)
pub fn read_xor_token(path: &Path) -> Result<String> {
    // 1. Read raw binary file
    // 2. Generate XOR key from username
    // 3. XOR transform (reverses encryption)
    // 4. Deserialize JSON
    // 5. Extract github_token field
}

// Internal helpers (not public)
fn obfuscation_key() -> Vec<u8>
fn xor_transform(data: &[u8], key: &[u8]) -> Vec<u8>
```

**Important**: This module is **read-only** and used only for migration. Do not use for new credential storage.

#### Migration Implementation (`src/storage/native.rs`)

```rust
impl NativeStorage {
    pub fn new(file_path: PathBuf) -> Result<Self> {
        let method = detect_method();
        
        // Migration: Check if we need to migrate from old XOR format
        if !file_path.exists() {
            let old_path = file_path.with_file_name("credentials.json");
            if old_path.exists() {
                migrate_from_xor(&old_path, &file_path, method);
                // Note: migrate_from_xor always deletes old file, even on failure
            }
        }
        
        // Edge case: Both old and new exist → remove old one
        let old_path = file_path.with_file_name("credentials.json");
        if file_path.exists() && old_path.exists() {
            if let Err(e) = std::fs::remove_file(&old_path) {
                tracing::warn!(
                    error = %e,
                    path = %old_path.display(),
                    "Failed to remove old credentials file (new format already exists)"
                );
            } else {
                tracing::info!("Removed old XOR credentials file (new format already exists)");
            }
        }
        
        Ok(Self { file_path, method })
    }
}

fn migrate_from_xor(old_path: &Path, new_path: &Path, method: StorageMethod) {
    tracing::info!(
        old = %old_path.display(),
        new = %new_path.display(),
        method = ?method,
        "Migrating credentials from XOR format to native encryption"
    );
    
    // 1. Try to read token from old XOR format (best-effort)
    let token_result = legacy::read_xor_token(old_path);
    
    // 2. If successful, write in new format
    if let Ok(token) = token_result {
        match NativeStorage {
            file_path: new_path.to_path_buf(),
            method,
        }.store_github_token(&token) {
            Ok(_) => {
                tracing::info!("Successfully migrated credentials to new format");
            }
            Err(e) => {
                tracing::warn!(
                    error = %e,
                    "Failed to store credentials in new format. \
                     Please run `copilot-adapter auth` to re-authenticate."
                );
            }
        }
    } else {
        tracing::warn!(
            error = %token_result.unwrap_err(),
            "Failed to read old XOR credentials. \
             Please run `copilot-adapter auth` to re-authenticate."
        );
    }
    
    // 3. ALWAYS delete old file (removes insecure XOR format)
    if let Err(e) = std::fs::remove_file(old_path) {
        tracing::warn!(
            error = %e,
            path = %old_path.display(),
            "Failed to delete old XOR credentials file"
        );
    } else {
        tracing::info!("Deleted old XOR credentials file");
    }
}
```

#### Migration Behavior Matrix

| Scenario | `github-copilot.json` | `credentials.json` | Action |
|----------|----------------------|--------------------|--------|
| **First run (new user)** | Not exists | Not exists | Normal flow: create new format on first auth |
| **Migration needed** | Not exists | Exists (XOR) | Read XOR → Write new format → **Delete old file** |
| **Already migrated** | Exists | Not exists | Normal flow: use new format |
| **Both exist** | Exists | Exists | Delete old file, use new format (no migration) |
| **Migration failed** | Not exists | Exists (corrupt) | Log warning, prompt re-auth, **delete old file anyway** |

#### Edge Cases and Error Handling

| Edge Case | Detection | Behavior |
|-----------|-----------|----------|
| **Corrupted XOR file** | `serde_json` deserialization fails | Log warning with clear message; prompt re-authentication; **delete old file** (stops insecure storage) |
| **Username changed** | XOR decrypt produces invalid JSON | Same as corrupted file (log + prompt re-auth + **delete old file**) |
| **Both files exist** | Both paths exist on entry | Delete old file without migration; log info message; use new format |
| **New format write fails** | `store_github_token()` returns error | Log error; prompt re-authentication; **delete old file anyway** |
| **Old file missing** | `old_path.exists()` returns false | No migration needed; normal flow |
| **Old file deletion fails** | `std::fs::remove_file()` returns error | Log warning; migration may have succeeded; user should manually delete |
| **Permission denied (read)** | `std::fs::read()` fails with permission error | Log error; prompt re-authentication; **attempt to delete old file anyway** |

#### Idempotency Guarantees

1. **Primary idempotency**: If `github-copilot.json` exists, skip migration entirely (checked at entry)
2. **Secondary idempotency**: If migration runs twice (race condition), second run skips because new file exists
3. **Edge case idempotency**: If both files exist, only removal is attempted (safe, no data loss)

#### Logging Strategy

**Info-level logs** (successful migration):
```
INFO  Migrating credentials from XOR format to native encryption old=/path/credentials.json new=/path/github-copilot.json method=Dpapi
INFO  Successfully migrated credentials to new format
INFO  Deleted old XOR credentials file
```

**Warning-level logs** (migration failures):
```
WARN  Failed to read old XOR credentials. Please run `copilot-adapter auth` to re-authenticate. error="Failed to parse credentials file. This can happen if your OS username changed..."
WARN  Failed to store credentials in new format. Please run `copilot-adapter auth` to re-authenticate. error="..."
WARN  Failed to delete old XOR credentials file error="Permission denied" path=/path/credentials.json
```

**Info-level logs** (edge case: both files exist):
```
INFO  Removed old XOR credentials file (new format already exists)
```

#### Testing Requirements

Add to unit tests (see Testing Strategy section):

1. **Migration success**: Create old XOR file → construct `NativeStorage` → verify new file created with correct format → verify old file deleted
2. **Migration idempotent**: Run migration twice → verify second run is no-op
3. **Edge case both files**: Create both formats → construct `NativeStorage` → verify old file deleted
4. **Corrupted XOR**: Create invalid XOR file → verify graceful failure with warning → verify old file deleted anyway
5. **Username changed**: Mock username change → verify XOR decrypt fails gracefully → verify old file deleted
6. **Old file deletion fails**: Mock filesystem error on delete → verify migration proceeds with warning
7. **Permission denied**: Mock permission error → verify clear error message → verify deletion attempted

#### Profile Isolation

Each profile migrates independently:
- Profile `default`: `~/.copilot-adapter/profiles/default/credentials.json` → `github-copilot.json`
- Profile `work`: `~/.copilot-adapter/profiles/work/credentials.json` → `github-copilot.json`

Migration state is per-profile (no shared migration marker file).

---

## Design Decisions

| Decision | Rationale |
|----------|-----------|
| Single `NativeStorage` struct | Eliminates dual-backend complexity; platform detection is internal |
| DPAPI on Windows (not Credential Manager) | DPAPI encrypts arbitrary data with user's credentials; Credential Manager is designed for username/password pairs and has size limits |
| File-based on Windows, keyring on macOS/Linux | DPAPI naturally produces encrypted files; macOS/Linux keychains are the standard secure storage |
| Sentinel JSON for keyring mode | Makes storage mechanism inspectable; avoids probing keyring on every `get_github_token()` call |
| Refuse insecure storage (no plaintext fallback) | Never store secrets without proper encryption; XOR and plaintext both give a false sense of security |
| Read by `"storage"` field, not detected method | Prevents silent data loss when keyring becomes unavailable after initial store |
| Profile-scoped keyring keys | Fixes cross-profile contamination bug present in current `--use-keyring` implementation |
| Rename to `github-copilot.json` | Less provocative filename; signals the file is not raw credentials |
| `version: 2` field | Enables future format changes without ambiguity |
| `base64` encoding for encrypted blob | Binary data in JSON requires encoding; base64 is standard and human-recognizable |
| **Delete old file, don't backup** | Priority is removing insecure XOR storage; if migration fails, prompt re-auth; simpler than managing backups |
| Migration in constructor | Automatic and transparent; no user action needed; happens on first access per profile |
| Extract XOR code to `legacy.rs` | Clean separation: read-only migration helpers vs. active storage implementation |
| Remove old file if both exist | Clear intent: new format takes precedence; prevents confusion; logged clearly |
| Graceful failure on corrupted XOR | Never crash on bad data; log warning and prompt re-auth; **always delete old file**; user-friendly error messages |

---

## File Changes Summary

| File | Change | Description |
|------|--------|-------------|
| `src/storage/dpapi.rs` | **New** | Windows DPAPI encrypt/decrypt FFI |
| `src/storage/native.rs` | **New** | Unified `NativeStorage` implementing `TokenStorage` |
| `src/storage/legacy.rs` | **New** | XOR read functions extracted from `file.rs` for migration; read-only, not for new code |
| `src/storage/mod.rs` | Modified | Simplified factory functions, export `legacy` module |
| `src/storage/file.rs` | **Deleted** | `FileStorage` struct removed; XOR functions moved to `legacy.rs` |
| `src/storage/keyring.rs` | **Deleted** | Merged into `native.rs` |
| `src/storage/windows_credential.rs` | **Deleted** | Replaced by DPAPI |
| `src/cli.rs` | Modified | Remove `use_keyring` from `Start` and `Auth` |
| `src/main.rs` | Modified | Remove `use_keyring` plumbing, simplify logout |
| `src/profile/types.rs` | Modified | `credentials_path()` → `"github-copilot.json"` |
| `src/profile/migration.rs` | Modified | Update `credentials.json` references |
| `Cargo.toml` | Modified | Add `base64`; change `windows-sys` features |
| `tests/windows_credential_test.rs` | **Deleted** | Dead code |
| Test files | Modified | Rewrite for `NativeStorage`, remove `use_keyring` tests, add migration tests |
| `CLAUDE.md` | Modified | Update storage documentation |

---

## Testing Strategy

### Unit Tests
1. **DPAPI round-trip** (Windows only): encrypt → decrypt → verify match
2. **NativeStorage CRUD**: store → get → delete → verify gone
3. **Credential file format**: verify JSON is pretty-printed and contains correct fields
4. **Storage method detection**: verify correct method per platform
5. **Retrieval by storage field**: write file with `"storage": "dpapi"`, verify read dispatches correctly
6. **Unavailable storage**: mock no-keyring environment → verify `store_github_token()` returns clear error
7. **Migration from XOR**: create old format file → construct `NativeStorage` → verify auto-migration
8. **Migration idempotent**: run migration twice → verify second run is no-op
9. **Edge case both files**: create both old and new files → construct `NativeStorage` → verify old file removed
10. **Corrupted XOR file**: create invalid XOR credentials → verify graceful failure with warning
11. **Username changed**: mock username change → verify XOR decrypt fails gracefully
12. **Backup failure**: mock filesystem error on rename → verify migration succeeds anyway
13. **Permission denied**: mock permission error → verify clear error message

### Integration Tests
1. **Auth flow → credential persistence**: authenticate → restart → token loaded
2. **Profile isolation**: two profiles store different tokens, each retrieves its own
3. **Logout cleanup**: verify credential file and keyring entry both removed

### Manual E2E Tests
1. Windows: `auth` → verify `github-copilot.json` contains `"storage": "dpapi"` with base64 blob
2. macOS/Linux: `auth` → verify `github-copilot.json` contains `"storage": "keyring"`
3. Migration: place old XOR `credentials.json`, start → verify auto-migration creates `github-copilot.json` and **deletes** `credentials.json`
4. Edge case both files: create both `credentials.json` and `github-copilot.json` → start → verify `credentials.json` deleted
5. Corrupted migration: create corrupted `credentials.json` → start → verify warning logged, prompt for re-auth, **old file deleted**
6. New user flow: fresh install → `auth` → verify only `github-copilot.json` created

---

## Risk Assessment

| Risk | Impact | Probability | Mitigation |
|------|--------|-------------|------------|
| DPAPI fails on unusual Windows configurations | Medium | Very Low | DPAPI is available on all Windows since 2000; no known failure modes |
| Windows user profile reset makes DPAPI data unreadable | Low | Low | Clear error message prompting re-authentication |
| Keyring unavailable on headless Linux | Medium | Medium | Clear error message with instructions to install Secret Service provider; adapter still usable with `--skip-auth` for external token management |
| Old XOR migration fails (corrupted file, username changed) | Low | Low | Warning logged; user prompted to re-authenticate; **old file deleted** to stop insecure storage |
| Users confused by file rename | Low | Low | Migration is transparent; old file deleted; documentation updated |
| Migration race condition (concurrent adapter starts) | Very Low | Very Low | Idempotent design prevents data loss; second migration attempt is no-op |
| Old file deletion fails due to permission issues | Very Low | Low | Warning logged; user instructed to manually delete; adapter still works with new format |
| Both old and new files exist (partial migration) | Very Low | Low | Detected as edge case; old file deleted; new file used; logged clearly |
| User loses credentials if migration fails | Low | Medium | Acceptable trade-off; re-authentication is straightforward; priority is stopping insecure storage |

---

## Success Criteria

1. **Windows:** `github-copilot.json` created with `"storage": "dpapi"` after authentication
2. **macOS/Linux:** Token stored in OS keyring, sentinel JSON created with `"storage": "keyring"`
3. **No `--use-keyring` flag** in `copilot-adapter start --help` or `copilot-adapter auth --help`
4. **Migration:** Old `credentials.json` automatically deleted after migration attempt (successful or not)
5. **Migration idempotent:** Running adapter multiple times does not re-migrate
6. **Edge case handled:** If both old and new files exist, old file is deleted
7. **Graceful failure:** Corrupted XOR files log clear warning, prompt re-authentication, and **delete old file**
8. **All tests pass:** `cargo test` on all platforms, including migration-specific tests
9. **Security priority:** No insecure XOR credentials remain after first run with new adapter
5. **All tests pass:** `cargo test` on all platforms

---

## References

- [HOME-DIR-TOKEN.design.md](./HOME-DIR-TOKEN.design.md) — Previous storage design (file-first with XOR)
- [CONSOLIDATED.plan.md](./CONSOLIDATED.plan.md) — Previous implementation plan
- [Windows DPAPI documentation](https://learn.microsoft.com/en-us/windows/win32/api/dpapi/)
- [windows-sys crate — CryptProtectData](https://docs.rs/windows-sys/0.61.0/windows_sys/Win32/Security/Cryptography/fn.CryptProtectData.html)
- [keyring crate](https://docs.rs/keyring/3.6/)
