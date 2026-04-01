use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

use super::TokenStorage;

const CREDENTIALS_FILE: &str = "credentials.json";

/// Simple XOR-based obfuscation key derived from the machine.
/// This is NOT cryptographic encryption — it is obfuscation to prevent
/// casual reading of the file. The OS keyring should be preferred.
fn obfuscation_key() -> Vec<u8> {
    // Use a fixed key mixed with the username for per-user differentiation
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

fn xor_transform(data: &[u8], key: &[u8]) -> Vec<u8> {
    data.iter()
        .enumerate()
        .map(|(i, b)| b ^ key[i % key.len()])
        .collect()
}

#[derive(Serialize, Deserialize, Default)]
struct Credentials {
    github_token: Option<String>,
}

impl Credentials {
    /// Returns true if all credential fields are None (no data worth keeping).
    fn is_empty(&self) -> bool {
        self.github_token.is_none()
    }
}

/// Returns the default credentials path: `~/.copilot-adapter/credentials.json`.
///
/// Uses the same base directory as the status module (`~/.copilot-adapter/`),
/// falling back to the OS temp directory if the home directory is unavailable.
pub fn get_credentials_path() -> PathBuf {
    crate::daemon::status::get_base_dir().join(CREDENTIALS_FILE)
}

/// Returns the legacy platform-specific credentials path used before the
/// migration to `~/.copilot-adapter/credentials.json`.
///
/// - **Windows:** `%APPDATA%\copilot-adapter\credentials.json`
/// - **macOS:** `~/Library/Application Support/copilot-adapter/credentials.json`
/// - **Linux:** `$XDG_CONFIG_HOME/copilot-adapter/credentials.json` or `~/.config/copilot-adapter/credentials.json`
pub fn get_legacy_credentials_path() -> Option<PathBuf> {
    let config_dir = dirs_sys_config_dir()?;
    Some(config_dir.join("copilot-adapter").join(CREDENTIALS_FILE))
}

/// Migrate credentials from the legacy platform-specific path to the new
/// default path (`~/.copilot-adapter/credentials.json`) if:
/// 1. The new path does not already contain a credentials file
/// 2. The legacy path does contain a credentials file
///
/// The migration copies (not moves) the file so the legacy location remains
/// readable by older versions. This is a no-op if the new file already exists
/// or no legacy file is found.
pub fn migrate_if_needed(new_path: &std::path::Path) {
    // Don't migrate if the new location already has credentials
    if new_path.exists() {
        return;
    }

    let legacy_path = match get_legacy_credentials_path() {
        Some(p) => p,
        None => return,
    };

    // Skip if legacy path equals new path (no migration needed)
    if legacy_path == new_path {
        return;
    }

    migrate_from_to(&legacy_path, new_path);
}

/// Copy a credentials file from `source` to `dest`, creating parent
/// directories as needed and setting restrictive permissions on Unix.
///
/// This is the inner migration logic extracted for testability. It is a
/// no-op if `source` does not exist.
pub fn migrate_from_to(source: &std::path::Path, dest: &std::path::Path) {
    if !source.exists() {
        return;
    }

    // Ensure parent directory of dest exists
    if let Some(parent) = dest.parent() {
        if fs::create_dir_all(parent).is_err() {
            tracing::warn!(
                "Failed to create directory for credential migration: {}",
                parent.display()
            );
            return;
        }
    }

    // Copy the source file to the destination
    match fs::copy(source, dest) {
        Ok(_) => {
            tracing::info!(
                legacy_path = %source.display(),
                new_path = %dest.display(),
                "Migrated credentials from legacy path"
            );

            // Best-effort: restrict file permissions on Unix
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let perms = std::fs::Permissions::from_mode(0o600);
                let _ = std::fs::set_permissions(dest, perms);
            }
        }
        Err(e) => {
            tracing::warn!(
                error = %e,
                legacy_path = %source.display(),
                new_path = %dest.display(),
                "Failed to migrate credentials from legacy path"
            );
        }
    }
}

/// File-based token storage using an obfuscated JSON file.
///
/// Default path: `~/.copilot-adapter/credentials.json`.
pub struct FileStorage {
    path: PathBuf,
}

impl Default for FileStorage {
    fn default() -> Self {
        Self::new()
    }
}

impl FileStorage {
    pub fn new() -> Self {
        let path = get_credentials_path();
        migrate_if_needed(&path);
        Self { path }
    }

    /// Create a FileStorage with a custom path.
    ///
    /// Used for profile support and testing. Does NOT trigger migration.
    pub fn with_path(path: PathBuf) -> Self {
        Self { path }
    }

    fn read_credentials(&self) -> Result<Credentials> {
        if !self.path.exists() {
            return Ok(Credentials::default());
        }

        let raw =
            fs::read(&self.path).map_err(|e| anyhow!("Failed to read credentials file: {e}"))?;
        let key = obfuscation_key();
        let decrypted = xor_transform(&raw, &key);
        let creds: Credentials = serde_json::from_slice(&decrypted).map_err(|_| {
            anyhow!(
                "Failed to parse credentials file. This can happen if your OS username \
                 changed since credentials were stored. Please run `copilot-adapter auth` \
                 to re-authenticate."
            )
        })?;
        Ok(creds)
    }

    fn write_credentials(&self, creds: &Credentials) -> Result<()> {
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent)
                .map_err(|e| anyhow!("Failed to create config directory: {e}"))?;
        }

        let json = serde_json::to_vec(creds)
            .map_err(|e| anyhow!("Failed to serialize credentials: {e}"))?;
        let key = obfuscation_key();
        let encrypted = xor_transform(&json, &key);
        fs::write(&self.path, encrypted)
            .map_err(|e| anyhow!("Failed to write credentials file: {e}"))?;

        // Best-effort: restrict file permissions on Unix
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let perms = std::fs::Permissions::from_mode(0o600);
            let _ = std::fs::set_permissions(&self.path, perms);
        }

        Ok(())
    }
}

impl TokenStorage for FileStorage {
    fn store_github_token(&self, token: &str) -> Result<()> {
        let mut creds = self.read_credentials().unwrap_or_default();
        creds.github_token = Some(token.to_string());
        self.write_credentials(&creds)
    }

    fn get_github_token(&self) -> Result<String> {
        let creds = self.read_credentials()?;
        creds
            .github_token
            .ok_or_else(|| anyhow!("No GitHub token stored"))
    }

    fn delete_github_token(&self) -> Result<()> {
        if self.path.exists() {
            let mut creds = self.read_credentials().unwrap_or_default();
            creds.github_token = None;
            // If no credential fields remain, remove the file entirely
            if creds.is_empty() {
                let _ = fs::remove_file(&self.path);
            } else {
                self.write_credentials(&creds)?;
            }
        }
        Ok(())
    }
}

/// Cross-platform config directory resolution for legacy paths.
fn dirs_sys_config_dir() -> Option<PathBuf> {
    #[cfg(target_os = "windows")]
    {
        std::env::var("APPDATA").ok().map(PathBuf::from)
    }
    #[cfg(target_os = "macos")]
    {
        dirs::home_dir().map(|h| h.join("Library").join("Application Support"))
    }
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    {
        std::env::var("XDG_CONFIG_HOME")
            .ok()
            .map(PathBuf::from)
            .or_else(|| dirs::home_dir().map(|h| h.join(".config")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn file_storage_round_trip() {
        let dir = std::env::temp_dir().join(format!("copilot-adapter-test-{}", std::process::id()));
        let _ = fs::create_dir_all(&dir);
        let path = dir.join("test_credentials.json");

        let storage = FileStorage::with_path(path.clone());

        // Store
        storage.store_github_token("ghp_test123").unwrap();

        // Retrieve
        let token = storage.get_github_token().unwrap();
        assert_eq!(token, "ghp_test123");

        // Delete
        storage.delete_github_token().unwrap();
        assert!(storage.get_github_token().is_err());

        // Cleanup
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn file_storage_overwrite() {
        let dir = std::env::temp_dir().join(format!(
            "copilot-adapter-test-overwrite-{}",
            std::process::id()
        ));
        let _ = fs::create_dir_all(&dir);
        let path = dir.join("test_credentials.json");

        let storage = FileStorage::with_path(path.clone());

        storage.store_github_token("token1").unwrap();
        storage.store_github_token("token2").unwrap();
        assert_eq!(storage.get_github_token().unwrap(), "token2");

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn file_storage_no_file_returns_error() {
        let path = PathBuf::from("/tmp/nonexistent-copilot-adapter-test/creds.json");
        let storage = FileStorage::with_path(path);
        assert!(storage.get_github_token().is_err());
    }

    #[test]
    fn xor_roundtrip() {
        let key = obfuscation_key();
        let data = b"hello world";
        let encrypted = xor_transform(data, &key);
        let decrypted = xor_transform(&encrypted, &key);
        assert_eq!(decrypted, data);
    }

    #[test]
    fn credentials_is_empty_when_github_token_is_none() {
        let creds = Credentials { github_token: None };
        assert!(creds.is_empty());
    }

    #[test]
    fn credentials_is_not_empty_when_github_token_is_some() {
        let creds = Credentials {
            github_token: Some("ghp_test".to_string()),
        };
        assert!(!creds.is_empty());
    }

    #[test]
    fn corrupted_file_produces_username_error_message() {
        let dir = std::env::temp_dir().join(format!(
            "copilot-adapter-test-corrupt-{}",
            std::process::id()
        ));
        let _ = fs::create_dir_all(&dir);
        let path = dir.join("bad_credentials.json");

        // Write random bytes that will not decode to valid JSON
        fs::write(&path, b"this is not valid encrypted data at all").unwrap();

        let storage = FileStorage::with_path(path);
        let err = storage.get_github_token().unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("username") || msg.contains("re-authenticate"),
            "Error message should mention username or re-authenticate, got: {msg}"
        );

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn get_credentials_path_returns_home_dir_based_path() {
        let path = get_credentials_path();
        assert_eq!(path.file_name().unwrap(), "credentials.json");
        // Should be under ~/.copilot-adapter/
        let parent = path.parent().unwrap();
        assert!(
            parent.ends_with(".copilot-adapter"),
            "Expected parent to end with .copilot-adapter, got: {}",
            parent.display()
        );
    }

    #[test]
    fn with_path_uses_custom_path() {
        let custom = PathBuf::from("/custom/path/creds.json");
        let storage = FileStorage::with_path(custom.clone());
        // We can't read from it but verify the path was stored correctly
        // by checking that get_github_token returns a "no token" error
        // (the file doesn't exist), not a path-related error
        let err = storage.get_github_token().unwrap_err();
        assert!(
            err.to_string().contains("No GitHub token"),
            "Expected 'No GitHub token' error for custom path, got: {}",
            err
        );
    }

    #[test]
    fn migrate_from_to_copies_credentials() {
        let dir = std::env::temp_dir().join(format!(
            "copilot-adapter-test-migrate-{}",
            std::process::id()
        ));
        let legacy_dir = dir.join("legacy");
        let new_dir = dir.join("new");
        let _ = fs::create_dir_all(&legacy_dir);
        // Deliberately do NOT create new_dir — migrate_from_to should create it

        let legacy_path = legacy_dir.join("credentials.json");
        let new_path = new_dir.join("credentials.json");

        // Create a credentials file at the legacy location
        let legacy_storage = FileStorage::with_path(legacy_path.clone());
        legacy_storage.store_github_token("ghp_legacy_token").unwrap();
        assert!(legacy_path.exists());

        // Run the actual migration function
        migrate_from_to(&legacy_path, &new_path);

        // Verify the new directory was created and the file was copied
        assert!(new_path.exists(), "migrate_from_to should create the destination file");

        // Verify the new file can be read back correctly
        let new_storage = FileStorage::with_path(new_path.clone());
        let token = new_storage.get_github_token().unwrap();
        assert_eq!(token, "ghp_legacy_token");

        // Legacy file should still exist (copy, not move)
        assert!(legacy_path.exists());

        // Cleanup
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn migrate_from_to_no_op_when_source_missing() {
        let dir = std::env::temp_dir().join(format!(
            "copilot-adapter-test-migrate-nosrc-{}",
            std::process::id()
        ));
        let _ = fs::create_dir_all(&dir);
        let missing_source = dir.join("nonexistent.json");
        let dest = dir.join("dest.json");

        // Should be a no-op when source does not exist
        migrate_from_to(&missing_source, &dest);
        assert!(!dest.exists(), "Destination should not be created when source is missing");

        // Cleanup
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn migrate_if_needed_skips_when_new_exists() {
        let dir = std::env::temp_dir().join(format!(
            "copilot-adapter-test-migrate-skip-{}",
            std::process::id()
        ));
        let _ = fs::create_dir_all(&dir);
        let new_path = dir.join("credentials.json");

        // Create a file at new_path first
        fs::write(&new_path, b"existing").unwrap();

        // migrate_if_needed should be a no-op because new_path already exists
        migrate_if_needed(&new_path);

        // File content should be unchanged
        let content = fs::read(&new_path).unwrap();
        assert_eq!(content, b"existing");

        // Cleanup
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn migrate_if_needed_no_op_when_no_legacy() {
        let dir = std::env::temp_dir().join(format!(
            "copilot-adapter-test-migrate-noop-{}",
            std::process::id()
        ));
        let _ = fs::create_dir_all(&dir);
        let new_path = dir.join("credentials.json");

        // No legacy file exists, no new file exists
        migrate_if_needed(&new_path);

        // Nothing should have been created
        assert!(!new_path.exists());

        // Cleanup
        let _ = fs::remove_dir_all(&dir);
    }
}
