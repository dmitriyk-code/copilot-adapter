//! Native credential storage with platform-appropriate encryption.
//!
//! Provides a unified [`NativeStorage`] backend that automatically selects the
//! best credential protection mechanism for the current platform:
//!
//! - **Windows:** DPAPI — token encrypted with user credentials, stored in JSON file
//! - **macOS/Linux:** OS keyring (Keychain / Secret Service) — token in keyring,
//!   sentinel JSON file on disk
//!
//! The on-disk file is `github-copilot.json` (version 2 format), replacing the old
//! XOR-obfuscated `credentials.json`. Automatic migration from the old format is
//! performed on first access.

use anyhow::{anyhow, Context, Result};
use base64::{engine::general_purpose::STANDARD as BASE64, Engine};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

use super::legacy;
use super::TokenStorage;

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

    /// Create a new NativeStorage instance.
    ///
    /// Detects the best storage method for the current platform and performs
    /// automatic migration from the old XOR format (`credentials.json`) if needed.
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

impl super::TokenStorage for NativeStorage {
    fn store_github_token(&self, token: &str) -> Result<()> {
        // Ensure parent directory exists
        if let Some(parent) = self.file_path.parent() {
            std::fs::create_dir_all(parent)
                .context("Failed to create credentials directory")?;
        }

        match self.method {
            StorageMethod::Dpapi => {
                #[cfg(target_os = "windows")]
                {
                    let encrypted = crate::storage::dpapi::encrypt(token.as_bytes())
                        .context("Failed to encrypt token with DPAPI")?;
                    let encoded = BASE64.encode(&encrypted);

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
            StorageMethod::Unavailable => Err(anyhow!(
                "No secure credential storage available. \
                 On Linux, install and start a Secret Service provider \
                 (e.g., GNOME Keyring, KDE Wallet, or `pass`). \
                 Then re-run `copilot-adapter auth`."
            )),
        }
    }

    fn get_github_token(&self) -> Result<String> {
        // Read the credential file to determine storage method
        let data = std::fs::read_to_string(&self.file_path)
            .context("No credentials found. Please run `copilot-adapter auth`.")?;

        let file: CredentialFile =
            serde_json::from_str(&data).context("Failed to parse credentials file")?;

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

                    let encrypted = BASE64
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

    fn delete_github_token(&self) -> Result<()> {
        // Read file to determine storage method (if it exists)
        if let Ok(data) = std::fs::read_to_string(&self.file_path) {
            if let Ok(file) = serde_json::from_str::<CredentialFile>(&data) {
                // Delete keyring entry if using keyring storage
                if file.storage == "keyring" {
                    #[cfg(not(target_os = "windows"))]
                    {
                        if let Ok(entry) = self.keyring_entry() {
                            // Ignore errors — entry might not exist
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
                // File doesn't exist — that's fine
                Ok(())
            }
            Err(e) => Err(anyhow::Error::from(e).context("Failed to delete credentials file")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::TokenStorage;
    use std::fs;

    /// Helper to create a temp directory unique to this test run.
    fn test_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "copilot-adapter-native-{}-{}",
            name,
            std::process::id()
        ));
        let _ = fs::create_dir_all(&dir);
        dir
    }

    // --- CredentialFile serialization tests ---

    #[test]
    fn credential_file_dpapi_serialization() {
        let file = CredentialFile {
            version: 2,
            storage: "dpapi".to_string(),
            github_token: Some("base64data".to_string()),
        };
        let json = serde_json::to_string_pretty(&file).unwrap();
        assert!(json.contains("\"version\": 2"));
        assert!(json.contains("\"storage\": \"dpapi\""));
        assert!(json.contains("\"github_token\": \"base64data\""));
    }

    #[test]
    fn credential_file_keyring_serialization() {
        let file = CredentialFile {
            version: 2,
            storage: "keyring".to_string(),
            github_token: None,
        };
        let json = serde_json::to_string_pretty(&file).unwrap();
        assert!(json.contains("\"version\": 2"));
        assert!(json.contains("\"storage\": \"keyring\""));
        // github_token should be omitted (skip_serializing_if)
        assert!(
            !json.contains("github_token"),
            "github_token should be omitted when None"
        );
    }

    #[test]
    fn credential_file_round_trip() {
        let original = CredentialFile {
            version: 2,
            storage: "dpapi".to_string(),
            github_token: Some("encrypted-blob".to_string()),
        };
        let json = serde_json::to_string(&original).unwrap();
        let parsed: CredentialFile = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.version, 2);
        assert_eq!(parsed.storage, "dpapi");
        assert_eq!(parsed.github_token.as_deref(), Some("encrypted-blob"));
    }

    #[test]
    fn credential_file_deserializes_without_github_token() {
        let json = r#"{"version": 2, "storage": "keyring"}"#;
        let file: CredentialFile = serde_json::from_str(json).unwrap();
        assert_eq!(file.version, 2);
        assert_eq!(file.storage, "keyring");
        assert!(file.github_token.is_none());
    }

    // --- StorageMethod tests ---

    #[test]
    fn storage_method_debug_and_clone() {
        let method = StorageMethod::Dpapi;
        let cloned = method;
        assert_eq!(format!("{:?}", cloned), "Dpapi");

        let method2 = StorageMethod::Keyring;
        assert_eq!(format!("{:?}", method2), "Keyring");

        let method3 = StorageMethod::Unavailable;
        assert_eq!(format!("{:?}", method3), "Unavailable");
    }

    #[test]
    fn storage_method_equality() {
        assert_eq!(StorageMethod::Dpapi, StorageMethod::Dpapi);
        assert_ne!(StorageMethod::Dpapi, StorageMethod::Keyring);
        assert_ne!(StorageMethod::Keyring, StorageMethod::Unavailable);
    }

    // --- Platform detection tests ---

    #[cfg(target_os = "windows")]
    #[test]
    fn detect_method_returns_dpapi_on_windows() {
        assert_eq!(NativeStorage::detect_method(), StorageMethod::Dpapi);
    }

    // --- Constructor tests ---

    #[test]
    fn new_creates_storage_instance() {
        let dir = test_dir("new-basic");
        let path = dir.join("github-copilot.json");

        let storage = NativeStorage::new(path.clone(), "default".to_string()).unwrap();
        assert_eq!(storage.file_path, path);
        assert_eq!(storage.profile_name, "default");

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn new_removes_old_file_when_both_exist() {
        let dir = test_dir("both-exist");
        let new_path = dir.join("github-copilot.json");
        let old_path = dir.join("credentials.json");

        // Create both files
        fs::write(&new_path, r#"{"version":2,"storage":"dpapi"}"#).unwrap();
        fs::write(&old_path, b"old-xor-data").unwrap();

        let _storage = NativeStorage::new(new_path.clone(), "default".to_string()).unwrap();

        // New file should still exist
        assert!(new_path.exists());
        // Old file should have been removed
        assert!(
            !old_path.exists(),
            "Old credentials.json should be removed when both exist"
        );

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn new_no_crash_when_no_files_exist() {
        let dir = test_dir("no-files");
        let path = dir.join("github-copilot.json");

        // Should not crash even when no files exist
        let storage = NativeStorage::new(path, "test-profile".to_string()).unwrap();
        assert_eq!(storage.profile_name, "test-profile");

        let _ = fs::remove_dir_all(&dir);
    }

    // --- Migration tests ---

    #[test]
    fn migrate_from_xor_deletes_old_file_even_on_corrupted_data() {
        let dir = test_dir("migrate-corrupt");
        let old_path = dir.join("credentials.json");
        let new_path = dir.join("github-copilot.json");

        // Write corrupted XOR data
        fs::write(&old_path, b"not-valid-xor-data").unwrap();

        NativeStorage::migrate_from_xor(
            &old_path,
            &new_path,
            StorageMethod::Dpapi,
            "default",
        );

        // Old file should be deleted regardless
        assert!(
            !old_path.exists(),
            "Old file should always be deleted during migration"
        );

        let _ = fs::remove_dir_all(&dir);
    }

    // --- TokenStorage trait tests (DPAPI path, Windows only) ---

    #[cfg(target_os = "windows")]
    #[test]
    fn dpapi_store_get_round_trip() {
        let dir = test_dir("dpapi-roundtrip");
        let path = dir.join("github-copilot.json");

        let storage = NativeStorage {
            file_path: path.clone(),
            method: StorageMethod::Dpapi,
            profile_name: "default".to_string(),
        };

        storage.store_github_token("ghp_test_dpapi_123").unwrap();

        // Verify file exists and is readable JSON
        let content = fs::read_to_string(&path).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&content).unwrap();
        assert_eq!(parsed["version"], 2);
        assert_eq!(parsed["storage"], "dpapi");
        assert!(parsed["github_token"].is_string());

        // Retrieve
        let token = storage.get_github_token().unwrap();
        assert_eq!(token, "ghp_test_dpapi_123");

        let _ = fs::remove_dir_all(&dir);
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn dpapi_store_overwrites_existing() {
        let dir = test_dir("dpapi-overwrite");
        let path = dir.join("github-copilot.json");

        let storage = NativeStorage {
            file_path: path.clone(),
            method: StorageMethod::Dpapi,
            profile_name: "default".to_string(),
        };

        storage.store_github_token("token1").unwrap();
        storage.store_github_token("token2").unwrap();
        assert_eq!(storage.get_github_token().unwrap(), "token2");

        let _ = fs::remove_dir_all(&dir);
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn dpapi_delete_removes_file() {
        let dir = test_dir("dpapi-delete");
        let path = dir.join("github-copilot.json");

        let storage = NativeStorage {
            file_path: path.clone(),
            method: StorageMethod::Dpapi,
            profile_name: "default".to_string(),
        };

        storage.store_github_token("to-be-deleted").unwrap();
        assert!(path.exists());

        storage.delete_github_token().unwrap();
        assert!(!path.exists());

        let _ = fs::remove_dir_all(&dir);
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn dpapi_delete_idempotent() {
        let dir = test_dir("dpapi-delete-idem");
        let path = dir.join("github-copilot.json");

        let storage = NativeStorage {
            file_path: path,
            method: StorageMethod::Dpapi,
            profile_name: "default".to_string(),
        };

        // Deleting when nothing exists should be fine
        storage.delete_github_token().unwrap();
        storage.delete_github_token().unwrap();

        let _ = fs::remove_dir_all(&dir);
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn dpapi_get_no_file_returns_error() {
        let dir = test_dir("dpapi-no-file");
        let path = dir.join("github-copilot.json");

        let storage = NativeStorage {
            file_path: path,
            method: StorageMethod::Dpapi,
            profile_name: "default".to_string(),
        };

        let err = storage.get_github_token().unwrap_err();
        assert!(
            err.to_string().contains("No credentials found"),
            "Expected 'No credentials found' error, got: {}",
            err
        );

        let _ = fs::remove_dir_all(&dir);
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn dpapi_get_unsupported_version() {
        let dir = test_dir("dpapi-bad-version");
        let path = dir.join("github-copilot.json");

        let file = CredentialFile {
            version: 99,
            storage: "dpapi".to_string(),
            github_token: Some("data".to_string()),
        };
        let json = serde_json::to_string_pretty(&file).unwrap();
        fs::write(&path, json).unwrap();

        let storage = NativeStorage {
            file_path: path,
            method: StorageMethod::Dpapi,
            profile_name: "default".to_string(),
        };

        let err = storage.get_github_token().unwrap_err();
        assert!(
            err.to_string().contains("Unsupported credentials file version"),
            "Expected version error, got: {}",
            err
        );

        let _ = fs::remove_dir_all(&dir);
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn dpapi_get_unknown_storage_method() {
        let dir = test_dir("dpapi-unknown-method");
        let path = dir.join("github-copilot.json");

        fs::write(
            &path,
            r#"{"version":2,"storage":"magic","github_token":"data"}"#,
        )
        .unwrap();

        let storage = NativeStorage {
            file_path: path,
            method: StorageMethod::Dpapi,
            profile_name: "default".to_string(),
        };

        let err = storage.get_github_token().unwrap_err();
        assert!(
            err.to_string().contains("Unknown storage method"),
            "Expected unknown method error, got: {}",
            err
        );

        let _ = fs::remove_dir_all(&dir);
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn dpapi_get_missing_token_field() {
        let dir = test_dir("dpapi-missing-token");
        let path = dir.join("github-copilot.json");

        // DPAPI storage but no github_token field
        fs::write(&path, r#"{"version":2,"storage":"dpapi"}"#).unwrap();

        let storage = NativeStorage {
            file_path: path,
            method: StorageMethod::Dpapi,
            profile_name: "default".to_string(),
        };

        let err = storage.get_github_token().unwrap_err();
        assert!(
            err.to_string().contains("missing token field"),
            "Expected missing token error, got: {}",
            err
        );

        let _ = fs::remove_dir_all(&dir);
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn dpapi_get_invalid_base64() {
        let dir = test_dir("dpapi-bad-b64");
        let path = dir.join("github-copilot.json");

        fs::write(
            &path,
            r#"{"version":2,"storage":"dpapi","github_token":"!!!not-base64!!!"}"#,
        )
        .unwrap();

        let storage = NativeStorage {
            file_path: path,
            method: StorageMethod::Dpapi,
            profile_name: "default".to_string(),
        };

        let err = storage.get_github_token().unwrap_err();
        assert!(
            err.to_string().contains("base64"),
            "Expected base64 error, got: {}",
            err
        );

        let _ = fs::remove_dir_all(&dir);
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn dpapi_store_creates_parent_directories() {
        let dir = test_dir("dpapi-nested");
        let path = dir.join("sub").join("dir").join("github-copilot.json");

        let storage = NativeStorage {
            file_path: path.clone(),
            method: StorageMethod::Dpapi,
            profile_name: "default".to_string(),
        };

        storage.store_github_token("ghp_nested").unwrap();
        assert!(path.exists());
        assert_eq!(storage.get_github_token().unwrap(), "ghp_nested");

        let _ = fs::remove_dir_all(&dir);
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn dpapi_unicode_token() {
        let dir = test_dir("dpapi-unicode");
        let path = dir.join("github-copilot.json");

        let storage = NativeStorage {
            file_path: path,
            method: StorageMethod::Dpapi,
            profile_name: "default".to_string(),
        };

        let unicode_token = "ghp_tëst_tökéñ_🔑";
        storage.store_github_token(unicode_token).unwrap();
        assert_eq!(storage.get_github_token().unwrap(), unicode_token);

        let _ = fs::remove_dir_all(&dir);
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn dpapi_file_is_human_readable_json() {
        let dir = test_dir("dpapi-readable");
        let path = dir.join("github-copilot.json");

        let storage = NativeStorage {
            file_path: path.clone(),
            method: StorageMethod::Dpapi,
            profile_name: "default".to_string(),
        };

        storage.store_github_token("ghp_readable").unwrap();

        let content = fs::read_to_string(&path).unwrap();
        // Pretty-printed JSON should have newlines
        assert!(content.contains('\n'), "JSON should be pretty-printed");
        // Should be valid JSON
        let _: serde_json::Value = serde_json::from_str(&content).unwrap();

        let _ = fs::remove_dir_all(&dir);
    }

    // --- Cross-platform mismatch tests ---

    #[cfg(target_os = "windows")]
    #[test]
    fn get_keyring_storage_on_windows_returns_error() {
        let dir = test_dir("keyring-on-windows");
        let path = dir.join("github-copilot.json");

        // Simulate a keyring credential file on Windows
        fs::write(&path, r#"{"version":2,"storage":"keyring"}"#).unwrap();

        let storage = NativeStorage {
            file_path: path,
            method: StorageMethod::Dpapi,
            profile_name: "default".to_string(),
        };

        let err = storage.get_github_token().unwrap_err();
        assert!(
            err.to_string().contains("keyring storage, but this is Windows"),
            "Expected cross-platform error, got: {}",
            err
        );

        let _ = fs::remove_dir_all(&dir);
    }

    #[cfg(not(target_os = "windows"))]
    #[test]
    fn get_dpapi_storage_on_non_windows_returns_error() {
        let dir = test_dir("dpapi-on-linux");
        let path = dir.join("github-copilot.json");

        // Simulate a DPAPI credential file on non-Windows
        fs::write(
            &path,
            r#"{"version":2,"storage":"dpapi","github_token":"data"}"#,
        )
        .unwrap();

        let storage = NativeStorage {
            file_path: path,
            method: StorageMethod::Keyring,
            profile_name: "default".to_string(),
        };

        let err = storage.get_github_token().unwrap_err();
        assert!(
            err.to_string()
                .contains("DPAPI storage, but this is not Windows"),
            "Expected cross-platform error, got: {}",
            err
        );

        let _ = fs::remove_dir_all(&dir);
    }

    // --- Unavailable storage tests ---

    #[test]
    fn unavailable_store_returns_error() {
        let dir = test_dir("unavailable-store");
        let path = dir.join("github-copilot.json");

        let storage = NativeStorage {
            file_path: path,
            method: StorageMethod::Unavailable,
            profile_name: "default".to_string(),
        };

        let err = storage.store_github_token("token").unwrap_err();
        assert!(
            err.to_string()
                .contains("No secure credential storage available"),
            "Expected unavailable error, got: {}",
            err
        );

        let _ = fs::remove_dir_all(&dir);
    }

    // --- Migration integration tests (Windows, uses DPAPI for store) ---

    #[cfg(target_os = "windows")]
    #[test]
    fn migration_from_xor_to_dpapi() {
        let dir = test_dir("migrate-xor-to-dpapi");
        let old_path = dir.join("credentials.json");
        let new_path = dir.join("github-copilot.json");

        // Write a valid XOR-encrypted token using the same key derivation as legacy
        let token = "ghp_migrated_token_456";
        let json = format!(r#"{{"github_token":"{}"}}"#, token);
        let key = xor_key_for_testing();
        let encrypted = xor_transform_for_testing(json.as_bytes(), &key);
        fs::write(&old_path, &encrypted).unwrap();

        // Create storage — should trigger migration
        let storage = NativeStorage::new(new_path.clone(), "default".to_string()).unwrap();

        // Old file should be gone
        assert!(
            !old_path.exists(),
            "Old credentials.json should be deleted after migration"
        );

        // New file should exist
        assert!(
            new_path.exists(),
            "New github-copilot.json should be created"
        );

        // Token should be retrievable
        let retrieved = storage.get_github_token().unwrap();
        assert_eq!(retrieved, token);

        let _ = fs::remove_dir_all(&dir);
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn migration_skipped_when_new_file_exists() {
        let dir = test_dir("migrate-skip");
        let old_path = dir.join("credentials.json");
        let new_path = dir.join("github-copilot.json");

        // Write old XOR file
        fs::write(&old_path, b"old-xor-data").unwrap();

        // Write new format file with a known token
        let storage_pre = NativeStorage {
            file_path: new_path.clone(),
            method: StorageMethod::Dpapi,
            profile_name: "default".to_string(),
        };
        storage_pre
            .store_github_token("ghp_existing_token")
            .unwrap();

        // Create storage — migration should NOT overwrite
        let storage = NativeStorage::new(new_path.clone(), "default".to_string()).unwrap();

        // Token should still be the existing one
        let token = storage.get_github_token().unwrap();
        assert_eq!(token, "ghp_existing_token");

        // But old file should still be cleaned up (edge case handler)
        assert!(
            !old_path.exists(),
            "Old file should be removed when both exist"
        );

        let _ = fs::remove_dir_all(&dir);
    }

    // --- Helper functions for XOR tests (mirrors legacy.rs key derivation) ---

    #[cfg(test)]
    fn xor_key_for_testing() -> Vec<u8> {
        let user = std::env::var("USERNAME")
            .or_else(|_| std::env::var("USER"))
            .unwrap_or_else(|_| "copilot-adapter-user".to_string());
        let mut key: Vec<u8> = b"copilot-adapter-storage-key-v1".to_vec();
        let key_len = key.len();
        for (i, b) in user.bytes().enumerate() {
            key[i % key_len] ^= b;
        }
        key
    }

    #[cfg(test)]
    fn xor_transform_for_testing(data: &[u8], key: &[u8]) -> Vec<u8> {
        data.iter()
            .enumerate()
            .map(|(i, &byte)| byte ^ key[i % key.len()])
            .collect()
    }
}
