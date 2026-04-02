//! Legacy XOR-obfuscated credential reader.
//!
//! This module provides **read-only** access to credentials stored in the old
//! XOR-obfuscated format (`credentials.json`). It exists solely to support
//! automatic migration to the new platform-native encryption backend.
//!
//! **Do not use this module for new credential storage.**
//!
//! The XOR scheme was a simple obfuscation — not cryptographic encryption —
//! that used the OS username as part of the key derivation.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::Path;

/// Old credential file format (version 1, XOR-obfuscated).
///
/// Uses a non-optional `github_token` field: if the stored value is `null` or
/// missing, deserialization intentionally fails (there is nothing to migrate).
#[derive(Serialize, Deserialize)]
struct LegacyCredentials {
    github_token: String,
}

/// Read a GitHub token from an old XOR-obfuscated credentials.json file.
///
/// Returns:
/// - `Ok(token)` if the token was successfully read and decoded
/// - `Err(...)` with a descriptive message if:
///   - The file doesn't exist or can't be read
///   - The file is corrupted or not valid XOR-obfuscated JSON
///   - The OS username has changed since the file was written (key mismatch)
///
/// **Important:** This function is read-only and used only for migration.
/// Do not use for new credential storage.
pub fn read_xor_token(path: &Path) -> Result<String> {
    let data = std::fs::read(path)
        .with_context(|| format!("Failed to read old credentials file: {}", path.display()))?;

    let key = obfuscation_key();
    let deobfuscated = xor_transform(&data, &key);

    let creds: LegacyCredentials = serde_json::from_slice(&deobfuscated).with_context(|| {
        format!(
            "Failed to parse credentials file '{}'. \
             This can happen if your OS username changed since credentials \
             were stored. Please run `copilot-adapter auth` to re-authenticate.",
            path.display()
        )
    })?;

    Ok(creds.github_token)
}

/// Generate the XOR obfuscation key using the same algorithm as the old
/// `FileStorage` implementation.
///
/// The key is a fixed prefix (`copilot-adapter-storage-key-v1`) with the
/// current OS username XOR-mixed into it. This must remain byte-identical
/// to the key produced by `src/storage/file.rs::obfuscation_key()`.
fn obfuscation_key() -> Vec<u8> {
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
    use std::fs;

    #[test]
    fn xor_reversible() {
        let data = b"test data 1234!@#$";
        let key = b"test key";
        let transformed = xor_transform(data, key);
        let restored = xor_transform(&transformed, key);
        assert_eq!(restored, data);
    }

    #[test]
    fn xor_empty_data() {
        let data: &[u8] = b"";
        let key = b"key";
        let transformed = xor_transform(data, key);
        assert!(transformed.is_empty());
    }

    #[test]
    fn xor_single_byte() {
        let data = b"A";
        let key = b"K";
        let transformed = xor_transform(data, key);
        assert_eq!(transformed, vec![b'A' ^ b'K']);
        let restored = xor_transform(&transformed, key);
        assert_eq!(restored, data);
    }

    #[test]
    fn xor_key_wraps_around() {
        let data = b"ABCDE"; // 5 bytes
        let key = b"XY"; // 2 bytes — key must wrap
        let transformed = xor_transform(data, key);
        assert_eq!(transformed.len(), 5);

        // Verify manual wrap: A^X, B^Y, C^X, D^Y, E^X
        assert_eq!(transformed[0], b'A' ^ b'X');
        assert_eq!(transformed[1], b'B' ^ b'Y');
        assert_eq!(transformed[2], b'C' ^ b'X');
        assert_eq!(transformed[3], b'D' ^ b'Y');
        assert_eq!(transformed[4], b'E' ^ b'X');
    }

    #[test]
    fn obfuscation_key_is_deterministic() {
        let key1 = obfuscation_key();
        let key2 = obfuscation_key();
        assert_eq!(key1, key2);
        assert!(!key1.is_empty());
    }

    #[test]
    fn read_xor_token_missing_file() {
        let path = std::path::Path::new("/nonexistent/path/credentials.json");
        let err = read_xor_token(path).unwrap_err();
        assert!(
            err.to_string().contains("Failed to read old credentials file"),
            "Expected 'Failed to read old credentials file' error, got: {err}"
        );
    }

    #[test]
    fn read_xor_token_corrupted_file() {
        let dir = std::env::temp_dir().join(format!(
            "copilot-adapter-legacy-corrupt-{}",
            std::process::id()
        ));
        let _ = fs::create_dir_all(&dir);
        let path = dir.join("credentials.json");

        // Write random bytes that won't decode to valid JSON
        fs::write(&path, b"definitely not valid XOR-encrypted JSON data").unwrap();

        let err = read_xor_token(&path).unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("re-authenticate") || msg.contains("username"),
            "Error should mention re-authenticate or username change, got: {msg}"
        );

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn read_xor_token_round_trip_with_file_storage_format() {
        // Simulate what FileStorage.write_credentials produces:
        // XOR-encrypt a JSON blob with the obfuscation key
        let dir = std::env::temp_dir().join(format!(
            "copilot-adapter-legacy-roundtrip-{}",
            std::process::id()
        ));
        let _ = fs::create_dir_all(&dir);
        let path = dir.join("credentials.json");

        let json = br#"{"github_token":"ghp_test_legacy_token_123"}"#;
        let key = obfuscation_key();
        let encrypted = xor_transform(json, &key);
        fs::write(&path, &encrypted).unwrap();

        let token = read_xor_token(&path).unwrap();
        assert_eq!(token, "ghp_test_legacy_token_123");

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn read_xor_token_null_github_token_fails() {
        // If the old file had github_token: null, deserialization should fail
        // because LegacyCredentials expects a String, not Option<String>
        let dir = std::env::temp_dir().join(format!(
            "copilot-adapter-legacy-null-{}",
            std::process::id()
        ));
        let _ = fs::create_dir_all(&dir);
        let path = dir.join("credentials.json");

        let json = br#"{"github_token":null}"#;
        let key = obfuscation_key();
        let encrypted = xor_transform(json, &key);
        fs::write(&path, &encrypted).unwrap();

        let err = read_xor_token(&path).unwrap_err();
        assert!(
            err.to_string().contains("re-authenticate"),
            "Expected re-authenticate guidance, got: {}",
            err
        );

        let _ = fs::remove_dir_all(&dir);
    }
}
