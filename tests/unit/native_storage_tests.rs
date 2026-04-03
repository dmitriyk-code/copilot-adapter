use copilot_adapter::storage::native::NativeStorage;
use copilot_adapter::storage::TokenStorage;
use std::fs;

/// Helper to create a temp directory unique to this test run.
fn test_dir(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "copilot-adapter-native-integ-{}-{}",
        name,
        std::process::id()
    ));
    let _ = fs::create_dir_all(&dir);
    dir
}

/// XOR obfuscation key (mirrors legacy.rs key derivation for test fixtures).
fn xor_key() -> Vec<u8> {
    let user = std::env::var("USERNAME")
        .or_else(|_| std::env::var("USER"))
        .unwrap_or_else(|_| "copilot-adapter-user".to_string());
    let mut key: Vec<u8> = b"copilot-adapter-storage-key-v1".to_vec();
    let klen = key.len();
    for (i, b) in user.bytes().enumerate() {
        key[i % klen] ^= b;
    }
    key
}

/// XOR transform (same as legacy — encrypt/decrypt are the same operation).
fn xor_transform(data: &[u8], key: &[u8]) -> Vec<u8> {
    data.iter()
        .enumerate()
        .map(|(i, &b)| b ^ key[i % key.len()])
        .collect()
}

// ============================================================
// Task 6.1 coverage: DPAPI round-trip (Windows only)
// ============================================================

#[cfg(target_os = "windows")]
#[test]
fn native_storage_dpapi_full_lifecycle() {
    let dir = test_dir("lifecycle");
    let path = dir.join("github-copilot.json");

    let storage = NativeStorage::new(path.clone(), "test-profile".to_string()).unwrap();

    // Initially no token
    assert!(storage.get_github_token().is_err());

    // Store
    storage.store_github_token("ghp_lifecycle_token").unwrap();

    // Retrieve
    let token = storage.get_github_token().unwrap();
    assert_eq!(token, "ghp_lifecycle_token");

    // Overwrite
    storage.store_github_token("ghp_updated_token").unwrap();
    let token = storage.get_github_token().unwrap();
    assert_eq!(token, "ghp_updated_token");

    // Delete
    storage.delete_github_token().unwrap();
    assert!(storage.get_github_token().is_err());

    // Delete again (idempotent)
    storage.delete_github_token().unwrap();

    // Verify file is gone
    assert!(!path.exists());

    let _ = fs::remove_dir_all(&dir);
}

// ============================================================
// Task 6.2: Unit tests — NativeStorage
// ============================================================

// --- Store / get / delete operations ---

#[cfg(target_os = "windows")]
#[test]
fn test_store_and_retrieve_token() {
    let dir = test_dir("store-retrieve");
    let file_path = dir.join("github-copilot.json");
    let storage = NativeStorage::new(file_path, "test".to_string()).unwrap();

    let token = "ghp_test123456";
    storage.store_github_token(token).unwrap();

    let retrieved = storage.get_github_token().unwrap();
    assert_eq!(retrieved, token);

    let _ = fs::remove_dir_all(&dir);
}

#[cfg(target_os = "windows")]
#[test]
fn test_delete_removes_token() {
    let dir = test_dir("delete-removes");
    let file_path = dir.join("github-copilot.json");
    let storage = NativeStorage::new(file_path.clone(), "test".to_string()).unwrap();

    storage.store_github_token("ghp_test").unwrap();
    storage.delete_github_token().unwrap();

    assert!(!file_path.exists());
    assert!(storage.get_github_token().is_err());

    let _ = fs::remove_dir_all(&dir);
}

// --- File format validation ---

#[cfg(target_os = "windows")]
#[test]
fn test_credential_file_format() {
    let dir = test_dir("file-format");
    let file_path = dir.join("github-copilot.json");
    let storage = NativeStorage::new(file_path.clone(), "test".to_string()).unwrap();

    storage.store_github_token("ghp_test").unwrap();

    let content = fs::read_to_string(&file_path).unwrap();
    let json: serde_json::Value = serde_json::from_str(&content).unwrap();

    assert_eq!(json["version"], 2);
    assert!(json["storage"].is_string());

    // On Windows, storage must be "dpapi" and token must be present (base64-encoded)
    assert_eq!(json["storage"], "dpapi");
    assert!(json["github_token"].is_string());

    // Pretty-printed should have newlines
    assert!(content.contains('\n'));

    let _ = fs::remove_dir_all(&dir);
}

#[cfg(target_os = "windows")]
#[test]
fn native_storage_file_is_valid_json() {
    let dir = test_dir("json-check");
    let path = dir.join("github-copilot.json");

    let storage = NativeStorage::new(path.clone(), "default".to_string()).unwrap();
    storage.store_github_token("ghp_json_test").unwrap();

    let content = fs::read_to_string(&path).unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&content).unwrap();

    assert_eq!(parsed["version"], 2);
    assert_eq!(parsed["storage"], "dpapi");
    assert!(parsed["github_token"].is_string());
    assert!(content.contains('\n'));

    let _ = fs::remove_dir_all(&dir);
}

// --- Migration scenarios ---

#[cfg(target_os = "windows")]
#[test]
fn test_migration_from_xor() {
    let dir = test_dir("xor-migration");
    let old_path = dir.join("credentials.json");
    let new_path = dir.join("github-copilot.json");

    // Create a valid old-format XOR-obfuscated file with a known token
    let json = br#"{"github_token":"ghp_old_xor_token"}"#;
    let key = xor_key();
    let encrypted = xor_transform(json, &key);
    fs::write(&old_path, &encrypted).unwrap();
    assert!(old_path.exists());

    // Create NativeStorage — should trigger migration
    let storage = NativeStorage::new(new_path.clone(), "default".to_string()).unwrap();

    // Old file MUST be deleted (security: remove insecure XOR storage)
    assert!(
        !old_path.exists(),
        "Old credentials.json should be deleted after migration"
    );

    // New file should exist and be readable
    assert!(new_path.exists());
    let token = storage.get_github_token().unwrap();
    assert_eq!(token, "ghp_old_xor_token");

    let _ = fs::remove_dir_all(&dir);
}

#[cfg(target_os = "windows")]
#[test]
fn test_migration_idempotent() {
    let dir = test_dir("migration-idempotent");
    let file_path = dir.join("github-copilot.json");

    // First creation (no old file to migrate)
    let storage1 = NativeStorage::new(file_path.clone(), "test".to_string()).unwrap();
    storage1.store_github_token("ghp_test").unwrap();

    // Second creation (new file already exists — migration should be a no-op)
    let storage2 = NativeStorage::new(file_path.clone(), "test".to_string()).unwrap();
    let token = storage2.get_github_token().unwrap();

    assert_eq!(token, "ghp_test");

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn test_both_files_exist_edge_case() {
    let dir = test_dir("both-exist-edge");

    let old_path = dir.join("credentials.json");
    let new_path = dir.join("github-copilot.json");

    // Create both old and new files
    fs::write(&old_path, b"old data").unwrap();
    fs::write(&new_path, b"new data").unwrap();

    let _storage = NativeStorage::new(new_path, "test".to_string()).unwrap();

    // Old file should be deleted
    assert!(
        !old_path.exists(),
        "Old credentials.json should be removed when both files exist"
    );

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn test_migration_corrupted_xor_deletes_old_file() {
    let dir = test_dir("corrupt-xor");
    let old_path = dir.join("credentials.json");
    let new_path = dir.join("github-copilot.json");

    // Write garbage that won't decode to valid JSON
    fs::write(&old_path, b"definitely not valid XOR data").unwrap();

    let _storage = NativeStorage::new(new_path.clone(), "test".to_string()).unwrap();

    // Old file must always be deleted (even if migration fails)
    assert!(
        !old_path.exists(),
        "Old credentials.json should be deleted even when corrupted"
    );

    let _ = fs::remove_dir_all(&dir);
}

// --- Constructor / profile tests ---

#[test]
fn native_storage_constructor_with_profiles() {
    let dir = test_dir("profiles");
    let path = dir.join("github-copilot.json");

    let storage = NativeStorage::new(path, "my-profile".to_string()).unwrap();
    drop(storage);

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn native_storage_no_crash_when_no_files() {
    let dir = test_dir("no-files");
    let path = dir.join("github-copilot.json");

    // Should not crash even when no files exist
    let _storage = NativeStorage::new(path, "test".to_string()).unwrap();

    let _ = fs::remove_dir_all(&dir);
}

#[cfg(target_os = "windows")]
#[test]
fn native_storage_get_returns_error_without_store() {
    let dir = test_dir("get-no-store");
    let path = dir.join("github-copilot.json");

    let storage = NativeStorage::new(path, "test".to_string()).unwrap();
    let err = storage.get_github_token().unwrap_err();
    assert!(
        err.to_string().contains("No credentials found"),
        "Expected 'No credentials found' error, got: {}",
        err
    );

    let _ = fs::remove_dir_all(&dir);
}
