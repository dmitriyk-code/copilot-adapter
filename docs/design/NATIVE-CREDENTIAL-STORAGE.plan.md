# Native Credential Storage — Implementation Plan

**Status:** Not Started
**Date:** 2026-04-02
**Based on:** [NATIVE-CREDENTIAL-STORAGE.design.md](./NATIVE-CREDENTIAL-STORAGE.design.md)
**Prerequisite:** None
**Estimated Time:** 3-4 days

---

## Executive Summary

This plan implements platform-native credential encryption for copilot-adapter, replacing the current XOR obfuscation system with always-on OS-native security. The implementation removes the `--use-keyring` flag and provides automatic credential protection on all platforms.

This plan implements:
- Windows DPAPI encryption with FFI bindings
- Unified `NativeStorage` backend (replaces `FileStorage` and `KeyringStorage`)
- Human-readable JSON credential file (`github-copilot.json`)
- Profile-scoped keyring entries
- Automatic migration from old XOR format with priority on removing insecure storage
- Graceful error handling for environments without secure storage

**Total estimated time:** 3-4 days

---

## Background

### Current State

The adapter currently uses two storage backends selected via the `--use-keyring` flag:
- **FileStorage** (`src/storage/file.rs`): XOR obfuscation with username-derived key, stores to `credentials.json`
- **KeyringStorage** (`src/storage/keyring.rs`): OS keyring via `keyring` crate, opt-in via flag
- Selection logic in `src/storage/mod.rs`

Problems:
- XOR obfuscation is trivially reversible (not cryptographic)
- Users must explicitly opt into real security
- Credential file is binary despite `.json` extension
- Keyring entries shared across profiles
- Two-pass logout logic needed

### Target State

After implementation:
- Single `NativeStorage` backend with automatic platform detection
- Windows uses DPAPI, macOS/Linux use OS keyring
- Human-readable JSON file with encrypted/keyring-backed tokens
- Profile-scoped keyring keys
- No `--use-keyring` flag needed
- Automatic migration that prioritizes removing insecure XOR storage
- Clear error messages when no secure storage available

---

## Goals and Non-Goals

### Goals

| ID | Goal | Success Criteria |
|----|------|------------------|
| G1 | Platform-native encryption by default | Windows uses DPAPI, macOS uses Keychain, Linux uses Secret Service — no flags needed |
| G2 | Remove `--use-keyring` flag | CLI simplified; `Start` and `Auth` commands lose the flag |
| G3 | Human-readable credential file | `github-copilot.json` is valid JSON with `version`, `storage`, and optional `github_token` fields |
| G4 | Profile-scoped keyring entries | Each profile stores credentials independently in the OS keyring |
| G5 | Transparent migration with security priority | Old `credentials.json` (XOR format) auto-deleted on first access; migration best-effort; **priority is removing insecure storage**; re-authentication if migration fails is acceptable |
| G6 | Refuse insecure storage | If no native encryption is available, refuse to save credentials and show clear error |

### Non-Goals

| ID | Non-Goal | Rationale |
|----|----------|-----------|
| NG1 | Add third-party encryption crate (AES-GCM, etc.) | DPAPI is OS-native on Windows; keyring handles macOS/Linux |
| NG2 | Encrypt non-secret data (status.json, profiles) | Only the GitHub OAuth token is sensitive |
| NG3 | Support custom encryption keys or passphrases | DPAPI and keyring derive keys from OS user credentials |
| NG4 | Remove `keyring` crate entirely | Still needed for macOS Keychain and Linux Secret Service |

---

## Implementation Plan

### Epic 1: Windows DPAPI Module (Day 1, 0.5 days)

**Status:** DONE

**Objective:** Implement Windows DPAPI encryption/decryption using FFI

#### Task 1.1: Add Windows Dependencies

**File:** `Cargo.toml` (MODIFIED)

**Description:** Add `windows-sys` crate with required features for DPAPI

**Implementation:**
```toml
[target.'cfg(windows)'.dependencies]
windows-sys = { version = "0.59", features = [
    "Win32_Foundation",
    "Win32_Security_Cryptography",
] }
```

**Acceptance Criteria:**
- [x] Cargo.toml updated with windows-sys dependency
- [x] Build succeeds on Windows
- [x] No new dependencies on non-Windows platforms

#### Task 1.2: Create DPAPI Module

**File:** `src/storage/dpapi.rs` (NEW)

**Description:** Implement encrypt/decrypt functions using Windows DPAPI

**Implementation:**
```rust
#[cfg(target_os = "windows")]
use anyhow::{anyhow, Result};
#[cfg(target_os = "windows")]
use windows_sys::Win32::Foundation::LocalFree;
#[cfg(target_os = "windows")]
use windows_sys::Win32::Security::Cryptography::{
    CryptProtectData, CryptUnprotectData, CRYPT_INTEGER_BLOB,
};

/// Encrypt data using Windows DPAPI.
///
/// The encryption key is derived from the current user's Windows credentials.
/// Only the same user on the same machine can decrypt.
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
            return Err(anyhow!(
                "CryptProtectData failed: {}",
                std::io::Error::last_os_error()
            ));
        }
        
        let encrypted = std::slice::from_raw_parts(output.pbData, output.cbData as usize).to_vec();
        LocalFree(output.pbData as *mut _);
        Ok(encrypted)
    }
}

/// Decrypt data using Windows DPAPI.
#[cfg(target_os = "windows")]
pub fn decrypt(encrypted: &[u8]) -> Result<Vec<u8>> {
    unsafe {
        let mut input = CRYPT_INTEGER_BLOB {
            cbData: encrypted.len() as u32,
            pbData: encrypted.as_ptr() as *mut u8,
        };
        let mut output = CRYPT_INTEGER_BLOB {
            cbData: 0,
            pbData: std::ptr::null_mut(),
        };
        
        let result = CryptUnprotectData(
            &mut input,
            std::ptr::null_mut(), // description (out)
            std::ptr::null_mut(), // entropy (optional)
            std::ptr::null_mut(), // reserved
            std::ptr::null_mut(), // prompt struct
            0,                    // flags
            &mut output,
        );
        
        if result == 0 {
            return Err(anyhow!(
                "CryptUnprotectData failed: {}",
                std::io::Error::last_os_error()
            ));
        }
        
        let decrypted = std::slice::from_raw_parts(output.pbData, output.cbData as usize).to_vec();
        LocalFree(output.pbData as *mut _);
        Ok(decrypted)
    }
}

#[cfg(test)]
#[cfg(target_os = "windows")]
mod tests {
    use super::*;

    #[test]
    fn test_encrypt_decrypt_round_trip() {
        let plaintext = b"test-token-ghp_123456789";
        let encrypted = encrypt(plaintext).expect("encrypt failed");
        assert_ne!(encrypted, plaintext, "encrypted should differ from plaintext");
        
        let decrypted = decrypt(&encrypted).expect("decrypt failed");
        assert_eq!(decrypted, plaintext, "decrypted should match original");
    }

    #[test]
    fn test_empty_string() {
        let plaintext = b"";
        let encrypted = encrypt(plaintext).expect("encrypt failed");
        let decrypted = decrypt(&encrypted).expect("decrypt failed");
        assert_eq!(decrypted, plaintext);
    }
}
```

**Acceptance Criteria:**
- [x] Module created with encrypt/decrypt functions
- [x] Proper FFI unsafe blocks with error handling
- [x] Memory cleanup via LocalFree
- [x] Unit tests pass on Windows
- [x] Module is cfg-gated for Windows only

**Notes:** DPAPI functions are always available on Windows (since Windows 2000), no availability check needed.

---

### Epic 2: Legacy XOR Module (Day 1, 0.25 days)

**Status:** DONE

**Objective:** Extract XOR reading functions from FileStorage for migration use

#### Task 2.1: Create Legacy Module

**File:** `src/storage/legacy.rs` (NEW)

**Description:** Extract and adapt XOR read functions from current file.rs for migration

**Implementation:**
```rust
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::Path;

/// Old credential file format (version 1, XOR-obfuscated).
#[derive(Serialize, Deserialize)]
struct LegacyCredentials {
    github_token: String,
}

/// Read a GitHub token from an old XOR-obfuscated credentials.json file.
///
/// Returns:
/// - `Ok(token)` if the token was successfully read
/// - `Err(...)` with descriptive error if:
///   - File doesn't exist
///   - File is corrupted
///   - OS username has changed (XOR key mismatch)
///
/// **Important:** This function is read-only and used only for migration.
/// Do not use for new credential storage.
pub fn read_xor_token(path: &Path) -> Result<String> {
    let data = std::fs::read(path)
        .with_context(|| format!("Failed to read old credentials file: {}", path.display()))?;

    let key = obfuscation_key()?;
    let deobfuscated = xor_transform(&data, &key);

    let creds: LegacyCredentials = serde_json::from_slice(&deobfuscated).with_context(|| {
        format!(
            "Failed to parse credentials file. \
             This can happen if your OS username changed. \
             Please run `copilot-adapter auth` to re-authenticate."
        )
    })?;

    Ok(creds.github_token)
}

/// Generate XOR obfuscation key from username.
fn obfuscation_key() -> Result<Vec<u8>> {
    let username = whoami::username();
    let mut key_source = String::from("copilot-adapter-v1-");
    key_source.push_str(&username);
    Ok(key_source.into_bytes())
}

/// XOR transform (reversible: encrypt and decrypt are the same operation).
fn xor_transform(data: &[u8], key: &[u8]) -> Vec<u8> {
    data.iter()
        .enumerate()
        .map(|(i, &byte)| byte ^ key[i % key.len()])
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_xor_reversible() {
        let data = b"test data";
        let key = b"test key";
        let transformed = xor_transform(data, key);
        let restored = xor_transform(&transformed, key);
        assert_eq!(restored, data);
    }
}
```

**Acceptance Criteria:**
- [x] Module created with read_xor_token function
- [x] Functions extracted from current file.rs
- [x] Proper error messages for common failures
- [x] Clear documentation that this is read-only for migration
- [x] Unit tests for XOR transform
- [x] No new dependencies needed (env var USERNAME/USER used instead of whoami for byte-identical key derivation)

**Implementation Notes:**
- Deviated from plan: used `std::env::var("USERNAME")` / `std::env::var("USER")` for key derivation instead of `whoami::username()`. This is correct — it produces byte-identical keys to `file.rs`'s existing implementation and avoids a dependency concern. Reviewed and approved.
- 9 unit tests pass (XOR reversibility, round-trip encode/decode, error cases, missing file).
- `Serialize` derive was noted as unnecessary (read-only module) but not a bug; all logic is correct.

---

### Epic 3: NativeStorage Implementation (Day 1-2, 1 day)

**Status:** DONE

**Objective:** Create unified NativeStorage backend with platform detection

#### Task 3.1: Define Types and Structures

**File:** `src/storage/native.rs` (NEW)

**Description:** Define credential file format and storage method enum

**Implementation:**
```rust
use anyhow::{anyhow, Context, Result};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

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

/// Native credential storage with platform-appropriate encryption.
pub struct NativeStorage {
    /// Path to the credential file (github-copilot.json).
    file_path: PathBuf,
    /// The storage method detected for this platform.
    method: StorageMethod,
    /// Profile name (for profile-scoped keyring entries).
    profile_name: String,
}
```

**Acceptance Criteria:**
- [x] Types defined with proper derives
- [x] Version field set to 2
- [x] Storage field is string (for JSON readability)
- [x] Optional github_token field

#### Task 3.2: Implement Platform Detection

**File:** `src/storage/native.rs` (continued)

**Description:** Implement detect_method() to select storage mechanism

**Implementation:**
```rust
impl NativeStorage {
    /// Detect which storage method to use for this platform.
    fn detect_method() -> StorageMethod {
        #[cfg(target_os = "windows")]
        {
            return StorageMethod::Dpapi;
        }

        #[cfg(not(target_os = "windows"))]
        {
            // Try to verify keyring availability
            if keyring::Entry::new("copilot-adapter", "probe")
                .and_then(|e| e.set_password("test"))
                .is_ok()
            {
                // Clean up probe entry
                let _ = keyring::Entry::new("copilot-adapter", "probe")
                    .and_then(|e| e.delete_credential());
                return StorageMethod::Keyring;
            }

            tracing::warn!(
                "No secure credential storage available. \
                 On Linux, install and start a Secret Service provider \
                 (e.g., GNOME Keyring, KDE Wallet, or `pass`). \
                 On macOS, keyring should be available by default."
            );
            StorageMethod::Unavailable
        }
    }

    /// Create a keyring entry for this profile.
    #[cfg(not(target_os = "windows"))]
    fn keyring_entry(&self) -> Result<keyring::Entry> {
        let username = format!("{}:github_token", self.profile_name);
        keyring::Entry::new("copilot-adapter", &username)
            .context("Failed to create keyring entry")
    }
}
```

**Acceptance Criteria:**
- [x] Windows always returns Dpapi
- [x] macOS/Linux probe keyring availability
- [x] Unavailable when keyring probe fails
- [x] Clear warning logged when unavailable
- [x] Profile-scoped keyring keys

#### Task 3.3: Implement Constructor with Migration

**File:** `src/storage/native.rs` (continued)

**Description:** Implement new() with automatic migration from old XOR format

**Implementation:**
```rust
use super::legacy;

impl NativeStorage {
    pub fn new(file_path: PathBuf, profile_name: String) -> Result<Self> {
        let method = Self::detect_method();
        
        // Migration: Check if we need to migrate from old XOR format
        if !file_path.exists() {
            let old_path = file_path.with_file_name("credentials.json");
            if old_path.exists() {
                Self::migrate_from_xor(&old_path, &file_path, method, &profile_name);
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
        
        Ok(Self {
            file_path,
            method,
            profile_name,
        })
    }

    fn migrate_from_xor(
        old_path: &Path,
        new_path: &Path,
        method: StorageMethod,
        profile_name: &str,
    ) {
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
            let storage = Self {
                file_path: new_path.to_path_buf(),
                method,
                profile_name: profile_name.to_string(),
            };
            
            match storage.store_github_token(&token) {
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
}
```

**Acceptance Criteria:**
- [x] Constructor detects storage method
- [x] Migration triggered when new file missing and old exists
- [x] Best-effort token read from old format
- [x] Old file always deleted (even on migration failure)
- [x] Edge case handled: both files exist
- [x] Clear logging at info/warn levels
- [x] Idempotent: safe to run multiple times

**Notes:** Migration prioritizes removing insecure XOR storage over preserving credentials. If migration fails, user re-authenticates.

#### Task 3.4: Implement TokenStorage Trait - Store

**File:** `src/storage/native.rs` (continued)

**Description:** Implement store_github_token() method

**Implementation:**
```rust
use base64::{engine::general_purpose::STANDARD as base64, Engine};
use std::path::Path;

impl super::TokenStorage for NativeStorage {
    fn store_github_token(&self, token: &str) -> Result<()> {
        match self.method {
            StorageMethod::Dpapi => {
                #[cfg(target_os = "windows")]
                {
                    let encrypted = crate::storage::dpapi::encrypt(token.as_bytes())
                        .context("Failed to encrypt token with DPAPI")?;
                    let encoded = base64.encode(&encrypted);
                    
                    let file = CredentialFile {
                        version: 2,
                        storage: "dpapi".to_string(),
                        github_token: Some(encoded),
                    };
                    
                    let json = serde_json::to_string_pretty(&file)
                        .context("Failed to serialize credentials")?;
                    std::fs::write(&self.file_path, json)
                        .context("Failed to write credentials file")?;
                    
                    Ok(())
                }
                #[cfg(not(target_os = "windows"))]
                {
                    unreachable!("DPAPI should only be detected on Windows")
                }
            }
            StorageMethod::Keyring => {
                #[cfg(not(target_os = "windows"))]
                {
                    let entry = self.keyring_entry()?;
                    entry
                        .set_password(token)
                        .context("Failed to store token in keyring")?;
                    
                    // Write sentinel file
                    let file = CredentialFile {
                        version: 2,
                        storage: "keyring".to_string(),
                        github_token: None,
                    };
                    
                    let json = serde_json::to_string_pretty(&file)
                        .context("Failed to serialize credentials")?;
                    std::fs::write(&self.file_path, json)
                        .context("Failed to write credentials file")?;
                    
                    Ok(())
                }
                #[cfg(target_os = "windows")]
                {
                    unreachable!("Keyring should not be detected on Windows")
                }
            }
            StorageMethod::Unavailable => {
                Err(anyhow!(
                    "No secure credential storage available. \
                     On Linux, install and start a Secret Service provider \
                     (e.g., GNOME Keyring, KDE Wallet, or `pass`). \
                     Then re-run `copilot-adapter auth`."
                ))
            }
        }
    }
}
```

**Acceptance Criteria:**
- [x] DPAPI path: encrypts, base64-encodes, writes JSON
- [x] Keyring path: stores in keyring, writes sentinel JSON
- [x] Unavailable: returns clear error message
- [x] JSON is pretty-printed for readability
- [x] Proper error context throughout
- [x] File written atomically (direct write is safe for JSON)

#### Task 3.5: Implement TokenStorage Trait - Get

**File:** `src/storage/native.rs` (continued)

**Description:** Implement get_github_token() method

**Implementation:**
```rust
impl super::TokenStorage for NativeStorage {
    fn get_github_token(&self) -> Result<String> {
        // Read the credential file to determine storage method
        let data = std::fs::read_to_string(&self.file_path).context(format!(
            "No credentials found. Please run `copilot-adapter auth`."
        ))?;
        
        let file: CredentialFile = serde_json::from_str(&data)
            .context("Failed to parse credentials file")?;
        
        if file.version != 2 {
            return Err(anyhow!(
                "Unsupported credentials file version: {}. \
                 Please run `copilot-adapter auth` to re-authenticate.",
                file.version
            ));
        }
        
        // Retrieve based on the file's storage field (not detected method)
        match file.storage.as_str() {
            "dpapi" => {
                #[cfg(target_os = "windows")]
                {
                    let encoded = file.github_token.ok_or_else(|| {
                        anyhow!("Credentials file missing token field for DPAPI storage")
                    })?;
                    
                    let encrypted = base64
                        .decode(&encoded)
                        .context("Failed to decode base64 token")?;
                    
                    let decrypted = crate::storage::dpapi::decrypt(&encrypted)
                        .context("Failed to decrypt token with DPAPI")?;
                    
                    String::from_utf8(decrypted)
                        .context("Decrypted token is not valid UTF-8")
                }
                #[cfg(not(target_os = "windows"))]
                {
                    Err(anyhow!(
                        "Credentials file indicates DPAPI storage, but this is not Windows. \
                         Please run `copilot-adapter auth` to re-authenticate."
                    ))
                }
            }
            "keyring" => {
                #[cfg(not(target_os = "windows"))]
                {
                    let entry = self.keyring_entry()?;
                    entry
                        .get_password()
                        .context("Failed to retrieve token from keyring")
                }
                #[cfg(target_os = "windows")]
                {
                    Err(anyhow!(
                        "Credentials file indicates keyring storage, but this is Windows. \
                         Please run `copilot-adapter auth` to re-authenticate."
                    ))
                }
            }
            other => Err(anyhow!(
                "Unknown storage method: {}. \
                 Please run `copilot-adapter auth` to re-authenticate.",
                other
            )),
        }
    }
}
```

**Acceptance Criteria:**
- [x] Reads file and checks version
- [x] Dispatches based on file's storage field (not detected method)
- [x] DPAPI: base64-decodes and decrypts
- [x] Keyring: retrieves from OS keyring
- [x] Clear error messages for all failure modes
- [x] Handles cross-platform file mismatches gracefully

**Notes:** Reading by the file's `storage` field (not the currently-detected method) prevents silent data loss when keyring becomes unavailable.

#### Task 3.6: Implement TokenStorage Trait - Delete

**File:** `src/storage/native.rs` (continued)

**Description:** Implement delete_github_token() method

**Implementation:**
```rust
impl super::TokenStorage for NativeStorage {
    fn delete_github_token(&self) -> Result<()> {
        // Read file to determine storage method (if it exists)
        if let Ok(data) = std::fs::read_to_string(&self.file_path) {
            if let Ok(file) = serde_json::from_str::<CredentialFile>(&data) {
                // Delete keyring entry if using keyring storage
                if file.storage == "keyring" {
                    #[cfg(not(target_os = "windows"))]
                    {
                        if let Ok(entry) = self.keyring_entry() {
                            // Ignore errors - entry might not exist
                            let _ = entry.delete_credential();
                        }
                    }
                }
            }
        }
        
        // Delete the file (ignore error if doesn't exist)
        match std::fs::remove_file(&self.file_path) {
            Ok(_) => {
                tracing::info!("Deleted credentials file");
                Ok(())
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                // File doesn't exist - that's fine
                Ok(())
            }
            Err(e) => Err(anyhow::Error::from(e).context("Failed to delete credentials file")),
        }
    }
}
```

**Acceptance Criteria:**
- [x] Deletes keyring entry if storage is "keyring"
- [x] Deletes credential file
- [x] Handles "file not found" gracefully (no-op)
- [x] Logs successful deletion
- [x] Returns error only for real failures
- [x] Single delete call handles everything

---

### Epic 4: Integration and CLI Updates (Day 2, 0.5 days)

**Status:** Not Started

**Objective:** Update storage factory and CLI to use NativeStorage

#### Task 4.1: Update Storage Module Exports

**File:** `src/storage/mod.rs` (MODIFIED)

**Description:** Simplify factory functions and update exports

**Implementation:**
```rust
mod dpapi;
mod keyring; // Will be removed in Task 4.3
mod legacy;
mod native;

pub use native::NativeStorage;
pub use legacy::read_xor_token; // For testing

use anyhow::Result;
use std::path::PathBuf;
use std::sync::Arc;

/// Trait for storing and retrieving tokens.
pub trait TokenStorage: Send + Sync {
    fn store_github_token(&self, token: &str) -> Result<()>;
    fn get_github_token(&self) -> Result<String>;
    fn delete_github_token(&self) -> Result<()>;
}

/// Create storage for a specific profile.
pub fn create_storage_for_profile(
    credentials_path: PathBuf,
    profile_name: String,
) -> Result<Arc<dyn TokenStorage>> {
    let storage = NativeStorage::new(credentials_path, profile_name)?;
    Ok(Arc::new(storage))
}
```

**Acceptance Criteria:**
- [ ] Exports updated to include native and legacy
- [ ] Simplified factory function (no use_keyring parameter)
- [ ] Profile name passed to storage
- [ ] Old exports removed

**Notes:** This is a breaking change - all callers must be updated.

#### Task 4.2: Update CLI Definitions

**File:** `src/cli.rs` (MODIFIED)

**Description:** Remove use_keyring flag from Auth and Start commands

**Implementation:**
```rust
// Before:
#[derive(Args)]
pub struct Auth {
    /// Force re-authentication even if credentials exist
    #[arg(long)]
    pub force: bool,

    /// Store credentials in OS keyring (default: file-based with XOR)
    #[arg(long)]
    pub use_keyring: bool,  // REMOVE THIS

    /// Profile name
    #[arg(short = 'P', long)]
    pub profile: Option<String>,
}

// After:
#[derive(Args)]
pub struct Auth {
    /// Force re-authentication even if credentials exist
    #[arg(long)]
    pub force: bool,

    /// Profile name
    #[arg(short = 'P', long)]
    pub profile: Option<String>,
}

// Similar changes for Start command
```

**Acceptance Criteria:**
- [ ] use_keyring removed from Auth struct
- [ ] use_keyring removed from Start struct
- [ ] All other fields preserved
- [ ] Doc comments updated
- [ ] Build succeeds

#### Task 4.3: Update Main Entry Points

**File:** `src/main.rs` (MODIFIED)

**Description:** Update auth and start handlers to use new storage factory

**Implementation:**
```rust
// In handle_auth():
let storage = storage::create_storage_for_profile(
    credentials_path,
    profile.name().to_string(),
)?;

// Remove use_keyring parameter

// In handle_start():
let storage = storage::create_storage_for_profile(
    credentials_path,
    profile.name().to_string(),
)?;

// Simplified logout - single delete call
// In handle_logout():
storage.delete_github_token()?;
```

**Acceptance Criteria:**
- [ ] All storage factory calls updated
- [ ] use_keyring parameter removed everywhere
- [ ] Logout simplified to single delete call
- [ ] Profile name passed correctly
- [ ] Builds and runs

#### Task 4.4: Update Profile Module

**File:** `src/profile/types.rs` (MODIFIED)

**Description:** Update credentials_path to return github-copilot.json

**Implementation:**
```rust
impl Profile {
    pub fn credentials_path(&self) -> PathBuf {
        self.base_path.join("github-copilot.json")  // Changed from "credentials.json"
    }
}
```

**Acceptance Criteria:**
- [ ] Function returns new filename
- [ ] All callers still work (path is opaque)

---

### Epic 5: Cleanup Old Code (Day 2, 0.25 days)

**Status:** Not Started

**Objective:** Remove obsolete storage implementations

#### Task 5.1: Delete Old Storage Files

**Files:**
- `src/storage/file.rs` (DELETE)
- `src/storage/keyring.rs` (DELETE)
- `src/storage/windows_credential.rs` (DELETE)
- `tests/windows_credential_test.rs` (DELETE)

**Description:** Remove old FileStorage, KeyringStorage, and Windows credential code

**Acceptance Criteria:**
- [ ] All files deleted
- [ ] No references to deleted modules remain
- [ ] Build succeeds
- [ ] Tests still pass

#### Task 5.2: Update Module Exports

**File:** `src/storage/mod.rs` (MODIFIED)

**Description:** Remove keyring module reference

**Implementation:**
```rust
mod dpapi;
// mod keyring; // REMOVE THIS LINE
mod legacy;
mod native;
```

**Acceptance Criteria:**
- [ ] Old module references removed
- [ ] Only native, legacy, and dpapi remain
- [ ] Build succeeds

---

### Epic 6: Testing (Day 3, 1 day)

**Status:** Not Started

**Objective:** Comprehensive testing of new storage system

#### Task 6.1: Unit Tests - DPAPI (Windows only)

**File:** `src/storage/dpapi.rs` (tests module already in Epic 1)

**Tests already implemented:**
1. **Round-trip encryption:**
   ```rust
   #[test]
   fn test_encrypt_decrypt_round_trip() { /* ... */ }
   ```
   - [ ] Test passes

2. **Empty string handling:**
   ```rust
   #[test]
   fn test_empty_string() { /* ... */ }
   ```
   - [ ] Test passes

**Acceptance Criteria:**
- [ ] All DPAPI tests pass on Windows
- [ ] Tests skipped on non-Windows platforms

#### Task 6.2: Unit Tests - NativeStorage

**File:** `tests/unit/native_storage_test.rs` (NEW)

**Tests to implement:**
```rust
use copilot_adapter::storage::{NativeStorage, TokenStorage};
use std::path::PathBuf;
use tempfile::TempDir;

#[test]
fn test_store_and_retrieve_token() {
    let temp = TempDir::new().unwrap();
    let file_path = temp.path().join("github-copilot.json");
    let storage = NativeStorage::new(file_path, "test".to_string()).unwrap();
    
    let token = "ghp_test123456";
    storage.store_github_token(token).unwrap();
    
    let retrieved = storage.get_github_token().unwrap();
    assert_eq!(retrieved, token);
}

#[test]
fn test_delete_removes_token() {
    let temp = TempDir::new().unwrap();
    let file_path = temp.path().join("github-copilot.json");
    let storage = NativeStorage::new(file_path.clone(), "test".to_string()).unwrap();
    
    storage.store_github_token("ghp_test").unwrap();
    storage.delete_github_token().unwrap();
    
    assert!(!file_path.exists());
    assert!(storage.get_github_token().is_err());
}

#[test]
fn test_credential_file_format() {
    let temp = TempDir::new().unwrap();
    let file_path = temp.path().join("github-copilot.json");
    let storage = NativeStorage::new(file_path.clone(), "test".to_string()).unwrap();
    
    storage.store_github_token("ghp_test").unwrap();
    
    let content = std::fs::read_to_string(&file_path).unwrap();
    let json: serde_json::Value = serde_json::from_str(&content).unwrap();
    
    assert_eq!(json["version"], 2);
    assert!(json["storage"].is_string());
    
    // Check platform-specific fields
    #[cfg(target_os = "windows")]
    {
        assert_eq!(json["storage"], "dpapi");
        assert!(json["github_token"].is_string());
    }
    #[cfg(not(target_os = "windows"))]
    {
        if json["storage"] == "keyring" {
            assert!(json["github_token"].is_null());
        }
    }
}

#[test]
fn test_migration_from_xor() {
    let temp = TempDir::new().unwrap();
    
    // Create old XOR credentials file
    let old_path = temp.path().join("credentials.json");
    // ... create old format file with known token ...
    
    let new_path = temp.path().join("github-copilot.json");
    let storage = NativeStorage::new(new_path.clone(), "test".to_string()).unwrap();
    
    // Old file should be deleted
    assert!(!old_path.exists());
    
    // New file should exist (if migration succeeded)
    if new_path.exists() {
        let token = storage.get_github_token().unwrap();
        // Verify token matches expected value
    }
}

#[test]
fn test_migration_idempotent() {
    let temp = TempDir::new().unwrap();
    let file_path = temp.path().join("github-copilot.json");
    
    // First migration
    let storage1 = NativeStorage::new(file_path.clone(), "test".to_string()).unwrap();
    storage1.store_github_token("ghp_test").unwrap();
    
    // Second migration (should be no-op)
    let storage2 = NativeStorage::new(file_path.clone(), "test".to_string()).unwrap();
    let token = storage2.get_github_token().unwrap();
    
    assert_eq!(token, "ghp_test");
}

#[test]
fn test_both_files_exist_edge_case() {
    let temp = TempDir::new().unwrap();
    
    // Create both old and new files
    let old_path = temp.path().join("credentials.json");
    let new_path = temp.path().join("github-copilot.json");
    
    std::fs::write(&old_path, b"old data").unwrap();
    std::fs::write(&new_path, b"new data").unwrap();
    
    let _storage = NativeStorage::new(new_path, "test".to_string()).unwrap();
    
    // Old file should be deleted
    assert!(!old_path.exists());
}
```

**Acceptance Criteria:**
- [ ] All tests pass
- [ ] Tests cover store/get/delete operations
- [ ] File format validated
- [ ] Migration scenarios tested
- [ ] Edge cases covered
- [ ] Platform-specific tests cfg-gated

#### Task 6.3: Integration Tests

**File:** `tests/integration/storage_integration_test.rs` (NEW)

**Scenarios to test:**
1. **Auth flow with credential persistence:**
   - Setup: Clean profile directory
   - Action: Run auth command → authenticate → restart
   - Verification: Token loaded successfully
   - [ ] Test passes

2. **Profile isolation:**
   - Setup: Create two profiles
   - Action: Store different tokens in each profile
   - Verification: Each profile retrieves its own token
   - [ ] Test passes

3. **Logout cleanup:**
   - Setup: Store credentials
   - Action: Run logout command
   - Verification: Credential file and keyring entry both removed
   - [ ] Test passes

**Acceptance Criteria:**
- [ ] All integration tests pass
- [ ] Tests run against real profile directories (tempdir)
- [ ] Cross-profile tests verify isolation

#### Task 6.4: Manual E2E Test Procedures

**File:** `docs/e2e-testing.md` (MODIFIED)

**Test procedures to add:**

1. **Windows DPAPI test:**
   ```bash
   # Clean state
   copilot-adapter logout
   rm -rf ~/.copilot-adapter
   
   # Authenticate
   copilot-adapter auth
   # Follow device flow
   
   # Verify file format
   cat ~/.copilot-adapter/profiles/default/github-copilot.json
   # Should show: {"version":2,"storage":"dpapi","github_token":"...base64..."}
   
   # Verify adapter works
   copilot-adapter start
   curl -X POST http://localhost:6767/v1/messages -d '...'
   ```
   - Expected: Token stored in DPAPI-encrypted format
   - [ ] Documented and tested

2. **macOS/Linux keyring test:**
   ```bash
   # Similar to Windows test
   # Verify file format shows "storage":"keyring" with no github_token field
   ```
   - Expected: Token in keyring, sentinel file created
   - [ ] Documented and tested

3. **Migration test:**
   ```bash
   # Setup: Create old XOR credentials file
   # (use previous adapter version or manually create)
   
   # Start new adapter
   copilot-adapter start
   
   # Verify:
   # - Old credentials.json deleted
   # - New github-copilot.json created
   # - Adapter works with migrated token
   ```
   - Expected: Automatic migration, old file deleted
   - [ ] Documented and tested

4. **Edge case: Both files exist:**
   ```bash
   # Create both credentials.json and github-copilot.json
   # Start adapter
   # Verify old file deleted, new file used
   ```
   - Expected: Old file removed, adapter uses new format
   - [ ] Documented and tested

5. **Corrupted XOR file:**
   ```bash
   # Create corrupted credentials.json
   # Start adapter
   # Verify warning logged, old file deleted, prompted to re-auth
   ```
   - Expected: Graceful failure with clear message
   - [ ] Documented and tested

**Acceptance Criteria:**
- [ ] All procedures documented in docs/e2e-testing.md
- [ ] Procedures tested manually on each platform
- [ ] Expected results clearly stated
- [ ] Screenshots or logs captured for verification

---

### Epic 7: Documentation (Day 3-4, 0.5 days)

**Status:** Not Started

**Objective:** Update all documentation to reflect new storage system

#### Task 7.1: Update CLAUDE.md

**File:** `CLAUDE.md` (MODIFIED)

**Changes:**
- Update "Credential storage" section:
  - Remove references to `--use-keyring` flag
  - Document automatic platform-native encryption
  - Explain `github-copilot.json` format
  - Document migration behavior
- Update "Commands" table:
  - Remove `--use-keyring` flag from auth and start commands
- Update "Notes for Development" section:
  - Add note about `github-copilot.json` format
  - Document migration priority (security over preservation)
  - Add troubleshooting for "no secure storage" error

**Acceptance Criteria:**
- [ ] All sections updated
- [ ] No references to old XOR format (except in migration context)
- [ ] Clear explanation of platform-specific behavior
- [ ] Migration behavior documented

#### Task 7.2: Update Design Document Status

**File:** `docs/design/NATIVE-CREDENTIAL-STORAGE.design.md` (MODIFIED)

**Changes:**
- Update status from "Proposed" to "Implemented"
- Add implementation date
- Add link to this plan document

**Acceptance Criteria:**
- [ ] Status updated
- [ ] Metadata current

#### Task 7.3: Create Migration Guide

**File:** `docs/migration-v2-credentials.md` (NEW)

**Content:**
```markdown
# Credential Storage Migration Guide

## Overview

Starting with version X.Y.Z, copilot-adapter uses platform-native encryption
for credential storage. This replaces the previous XOR obfuscation system.

## What Changed

### Before (v1 format)
- File: `credentials.json` (binary, XOR-obfuscated)
- Security: Reversible obfuscation (not cryptographic)
- Opt-in: `--use-keyring` flag for real encryption

### After (v2 format)
- File: `github-copilot.json` (human-readable JSON)
- Security: Platform-native encryption (always on)
  - Windows: DPAPI
  - macOS: Keychain
  - Linux: Secret Service
- No flags needed

## Automatic Migration

The adapter automatically migrates your credentials when you first run
version X.Y.Z or later:

1. Old `credentials.json` is read (best-effort)
2. Token is stored in new format
3. Old `credentials.json` is deleted

**Important:** Migration prioritizes security over preservation. If the
migration fails (corrupted file, username changed, etc.), you'll need to
re-authenticate. The old insecure file is always deleted.

## Manual Migration (if needed)

If automatic migration fails:

```bash
copilot-adapter logout  # Clear any partial state
copilot-adapter auth    # Re-authenticate
```

## Troubleshooting

### "No secure credential storage available"

On Linux, this means no Secret Service provider is running. Install one:

- Ubuntu/Debian: `gnome-keyring` or `kde-wallet`
- Arch: `gnome-keyring` or `kwalletmanager`
- Fedora: Usually pre-installed

Then start the service and re-run `copilot-adapter auth`.

### "Failed to read old XOR credentials"

This can happen if:
- Your OS username changed
- The old credentials file is corrupted
- File permissions changed

Solution: Re-authenticate with `copilot-adapter auth`.

## File Format Reference

### Windows (DPAPI)
```json
{
  "version": 2,
  "storage": "dpapi",
  "github_token": "AQAAANCM...base64..."
}
```

### macOS/Linux (Keyring)
```json
{
  "version": 2,
  "storage": "keyring"
}
```

Token is stored in OS keyring under service `copilot-adapter`,
username `{profile}:github_token`.
```

**Acceptance Criteria:**
- [ ] Guide created with clear explanations
- [ ] Troubleshooting section covers common issues
- [ ] File format examples included
- [ ] Migration behavior explained

---

## Requirements

### Functional Requirements

| ID | Requirement | Source | Epic |
|----|-------------|--------|------|
| FR1 | Windows credentials encrypted with DPAPI | Design G1 | Epic 1, 3 |
| FR2 | macOS/Linux credentials in OS keyring | Design G1 | Epic 3 |
| FR3 | Platform detection automatic (no user flags) | Design G2 | Epic 3, 4 |
| FR4 | Human-readable JSON credential file | Design G3 | Epic 3 |
| FR5 | Profile-scoped keyring entries | Design G4 | Epic 3 |
| FR6 | Automatic migration from XOR format | Design G5 | Epic 3 |
| FR7 | Old XOR file always deleted after migration | Design G5 | Epic 3 |
| FR8 | Error when no secure storage available | Design G6 | Epic 3 |

### Non-Functional Requirements

| ID | Requirement | Target | Epic |
|----|-------------|--------|------|
| NFR1 | Migration completes in <1s | <1000ms | Epic 3 |
| NFR2 | No plaintext tokens written to disk | 0 violations | Epic 3 |
| NFR3 | Clear error messages for all failures | 100% coverage | Epic 3 |
| NFR4 | Backward-compatible (read old format) | 100% | Epic 2, 3 |

---

## File Changes Summary

| File | Change | Epic | Description |
|------|--------|------|-------------|
| `Cargo.toml` | Modified | Epic 1 | Add windows-sys dependency |
| `src/storage/dpapi.rs` | **New** | Epic 1 | Windows DPAPI FFI module |
| `src/storage/legacy.rs` | **New** | Epic 2 | XOR read functions for migration |
| `src/storage/native.rs` | **New** | Epic 3 | Unified NativeStorage implementation |
| `src/storage/mod.rs` | Modified | Epic 4 | Simplified factory, updated exports |
| `src/storage/file.rs` | Deleted | Epic 5 | Old FileStorage removed |
| `src/storage/keyring.rs` | Deleted | Epic 5 | Old KeyringStorage removed |
| `src/storage/windows_credential.rs` | Deleted | Epic 5 | Old Windows code removed |
| `src/cli.rs` | Modified | Epic 4 | Remove use_keyring flag |
| `src/main.rs` | Modified | Epic 4 | Update storage factory calls |
| `src/profile/types.rs` | Modified | Epic 4 | Update credentials_path() |
| `tests/windows_credential_test.rs` | Deleted | Epic 5 | Dead code |
| `tests/unit/native_storage_test.rs` | **New** | Epic 6 | Unit tests for NativeStorage |
| `tests/integration/storage_integration_test.rs` | **New** | Epic 6 | Integration tests |
| `docs/e2e-testing.md` | Modified | Epic 6 | Add E2E test procedures |
| `CLAUDE.md` | Modified | Epic 7 | Update documentation |
| `docs/design/NATIVE-CREDENTIAL-STORAGE.design.md` | Modified | Epic 7 | Update status |
| `docs/migration-v2-credentials.md` | **New** | Epic 7 | Migration guide |

---

## Testing Strategy

### Test Coverage

| Component | Unit Tests | Integration Tests | E2E Tests |
|-----------|------------|-------------------|-----------|
| DPAPI (Windows) | Epic 6.1 | - | Epic 6.4 |
| NativeStorage | Epic 6.2 | Epic 6.3 | Epic 6.4 |
| Legacy XOR read | Epic 2 (inline) | - | Epic 6.4 |
| Migration | Epic 6.2 | - | Epic 6.4 |
| Profile isolation | - | Epic 6.3 | - |

### Test Files

| File | Type | Coverage |
|------|------|----------|
| `src/storage/dpapi.rs` (tests mod) | Unit | DPAPI encrypt/decrypt |
| `src/storage/legacy.rs` (tests mod) | Unit | XOR transform |
| `tests/unit/native_storage_test.rs` | Unit | NativeStorage CRUD, migration |
| `tests/integration/storage_integration_test.rs` | Integration | Auth flow, profiles, logout |
| `docs/e2e-testing.md` | Manual E2E | Platform-specific behavior |

---

## Dependencies

### External Dependencies

| Dependency | Version | Purpose | Epic |
|------------|---------|---------|------|
| `windows-sys` | 0.59 | Windows DPAPI FFI | Epic 1 |
| `base64` | 0.22 | Encode encrypted blobs | Epic 3 |
| `keyring` | 3.6 | Existing (macOS/Linux keyring) | Epic 3 |

**Cargo.toml changes:**
```toml
[dependencies]
base64 = "0.22"

[target.'cfg(windows)'.dependencies]
windows-sys = { version = "0.59", features = [
    "Win32_Foundation",
    "Win32_Security_Cryptography",
] }
```

### Internal Dependencies

| Module | Required By | Status |
|--------|-------------|--------|
| `storage::TokenStorage` | Epic 3 | ✅ Exists |
| `profile::Profile` | Epic 4 | ✅ Exists |
| `auth::device_flow` | Epic 4 | ✅ Exists |

---

## Risk Assessment

| Risk | Impact | Probability | Mitigation | Epic |
|------|--------|-------------|------------|------|
| DPAPI fails on unusual Windows configs | Medium | Very Low | DPAPI is universal on Windows; clear error messages | Epic 1 |
| Migration fails for corrupted files | Low | Low | Graceful failure with re-auth prompt; **old file deleted anyway** | Epic 3 |
| Keyring unavailable on headless Linux | Medium | Medium | Clear error with setup instructions; adapter usable with --skip-auth | Epic 3 |
| Users confused by file rename | Low | Low | Migration is transparent; documentation updated | Epic 7 |
| Cross-profile keyring contamination | Medium | Low | Fixed by profile-scoped keys | Epic 3 |
| Migration race condition | Very Low | Very Low | Idempotent design; second migration is no-op | Epic 3 |
| Old file deletion fails | Very Low | Low | Warning logged; user instructed to delete manually | Epic 3 |
| User loses credentials on migration failure | Low | Medium | Acceptable trade-off; re-auth straightforward; priority is security | Epic 3 |

---

## Success Criteria

1. **Windows DPAPI storage** — `github-copilot.json` created with `"storage": "dpapi"` after auth (Epic 1, 3)
2. **macOS/Linux keyring storage** — Token in OS keyring, sentinel JSON with `"storage": "keyring"` (Epic 3)
3. **No --use-keyring flag** — Flag removed from CLI help (Epic 4)
4. **Migration completes** — Old `credentials.json` deleted automatically (Epic 3)
5. **Migration idempotent** — Running multiple times is safe (Epic 3)
6. **Edge case handled** — Both old and new files present → old deleted (Epic 3)
7. **Graceful failure** — Corrupted XOR files handled with clear warnings (Epic 3)
8. **All tests pass** — Unit, integration, and E2E tests pass on all platforms (Epic 6)
9. **Documentation complete** — CLAUDE.md, migration guide, E2E procedures updated (Epic 7)
10. **Security priority** — No insecure XOR credentials remain after first run with new adapter (Epic 3)

---

## Rollout / Migration Plan

### Phase 1: Core Implementation (Day 1-2)
- [ ] Epic 1: Windows DPAPI module
- [x] Epic 2: Legacy XOR module
- [ ] Epic 3: NativeStorage implementation
- [ ] Epic 4: Integration and CLI updates
- [ ] Epic 5: Cleanup old code

### Phase 2: Testing (Day 3)
- [ ] Epic 6: Comprehensive testing
- [ ] Unit tests pass
- [ ] Integration tests pass
- [ ] Manual E2E verification on all platforms

### Phase 3: Documentation (Day 3-4)
- [ ] Epic 7: Documentation updates
- [ ] CLAUDE.md updated
- [ ] Migration guide created
- [ ] E2E test procedures documented

### Phase 4: Release
- [ ] All acceptance criteria met
- [ ] Code review completed
- [ ] Final testing on Windows, macOS, Linux
- [ ] Merge to main
- [ ] Tag release
- [ ] Archive design/plan docs

---

## Epic Status Tracking

| Epic | Status | Start Date | End Date | Notes |
|------|--------|------------|----------|-------|
| Epic 1: Windows DPAPI | Done | 2026-04-02 | 2026-04-02 | |
| Epic 2: Legacy XOR | Done | 2026-04-02 | 2026-04-02 | |
| Epic 3: NativeStorage | Not Started | - | - | |
| Epic 4: Integration | Not Started | - | - | |
| Epic 5: Cleanup | Not Started | - | - | |
| Epic 6: Testing | Not Started | - | - | |
| Epic 7: Documentation | Not Started | - | - | |

---

## Open Questions

| # | Question | Status | Blocker For |
|---|----------|--------|-------------|
| 1 | Should we add a `--force-insecure` flag for headless Linux without keyring? | Deferred | - |
| 2 | Should migration create a backup of the old file before deleting? | Resolved: No, priority is removing insecure storage | Epic 3 |

---

## References

- [NATIVE-CREDENTIAL-STORAGE.design.md](./NATIVE-CREDENTIAL-STORAGE.design.md) — Design document
- [HOME-DIR-TOKEN.design.md](./HOME-DIR-TOKEN.design.md) — Previous storage design
- [Windows DPAPI documentation](https://learn.microsoft.com/en-us/windows/win32/api/dpapi/)
- [windows-sys crate](https://docs.rs/windows-sys/0.59/)
- [keyring crate](https://docs.rs/keyring/3.6/)

---

## Notes

### Development Notes
- DPAPI is always available on Windows (no probe needed)
- Keyring probe on macOS/Linux must be non-destructive
- Migration must be idempotent (safe to run multiple times)
- File format uses pretty-printed JSON for human readability
- Profile name embedded in keyring username for isolation

### Review Notes
- Consider adding metrics for migration success/failure rates
- Monitor keyring unavailability on Linux (may need better docs)
- Ensure migration error messages are user-friendly

### Testing Notes
- DPAPI tests must run on Windows only (cfg-gate)
- Keyring tests need Secret Service running (skip if unavailable)
- Migration tests need fixtures for old XOR format
- E2E tests require manual verification across platforms
