use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

use super::TokenStorage;

const CONFIG_DIR: &str = "copilot-adapter";
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

/// Fallback token storage using an obfuscated JSON file.
pub struct FileStorage {
    path: PathBuf,
}

impl FileStorage {
    pub fn new() -> Self {
        let config_dir = dirs_config_dir().join(CONFIG_DIR);
        Self {
            path: config_dir.join(CREDENTIALS_FILE),
        }
    }

    /// Create a FileStorage with a custom path (for testing).
    pub fn with_path(path: PathBuf) -> Self {
        Self { path }
    }

    fn read_credentials(&self) -> Result<Credentials> {
        if !self.path.exists() {
            return Ok(Credentials::default());
        }

        let raw = fs::read(&self.path)
            .map_err(|e| anyhow!("Failed to read credentials file: {e}"))?;
        let key = obfuscation_key();
        let decrypted = xor_transform(&raw, &key);
        let creds: Credentials = serde_json::from_slice(&decrypted)
            .map_err(|_| anyhow!(
                "Failed to parse credentials file. This can happen if your OS username \
                 changed since credentials were stored. Please run `copilot-adapter auth` \
                 to re-authenticate."
            ))?;
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

/// Cross-platform config directory resolution.
fn dirs_config_dir() -> PathBuf {
    // Try standard config directories
    if let Some(config) = dirs_sys_config_dir() {
        return config;
    }
    // Fallback to home directory
    if let Some(home) = home_dir() {
        return home.join(".config");
    }
    PathBuf::from(".")
}

fn dirs_sys_config_dir() -> Option<PathBuf> {
    #[cfg(target_os = "windows")]
    {
        std::env::var("APPDATA").ok().map(PathBuf::from)
    }
    #[cfg(target_os = "macos")]
    {
        home_dir().map(|h| h.join("Library").join("Application Support"))
    }
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    {
        std::env::var("XDG_CONFIG_HOME")
            .ok()
            .map(PathBuf::from)
            .or_else(|| home_dir().map(|h| h.join(".config")))
    }
}

fn home_dir() -> Option<PathBuf> {
    std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .ok()
        .map(PathBuf::from)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn file_storage_round_trip() {
        let dir = std::env::temp_dir().join(format!(
            "copilot-adapter-test-{}",
            std::process::id()
        ));
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
}
