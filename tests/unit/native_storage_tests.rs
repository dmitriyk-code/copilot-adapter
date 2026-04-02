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

// --- DPAPI round-trip (Windows only) ---

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

#[cfg(target_os = "windows")]
#[test]
fn native_storage_file_is_valid_json() {
    let dir = test_dir("json-check");
    let path = dir.join("github-copilot.json");

    let storage = NativeStorage::new(path.clone(), "default".to_string()).unwrap();
    storage.store_github_token("ghp_json_test").unwrap();

    // Read file and verify it's valid, human-readable JSON
    let content = fs::read_to_string(&path).unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&content).unwrap();

    assert_eq!(parsed["version"], 2);
    assert_eq!(parsed["storage"], "dpapi");
    assert!(parsed["github_token"].is_string());

    // Pretty-printed should have newlines
    assert!(content.contains('\n'));

    let _ = fs::remove_dir_all(&dir);
}

#[cfg(target_os = "windows")]
#[test]
fn native_storage_migration_from_xor() {
    let dir = test_dir("xor-migration");
    let old_path = dir.join("credentials.json");
    let new_path = dir.join("github-copilot.json");

    // Simulate old XOR format using FileStorage
    let old_storage = copilot_adapter::storage::file::FileStorage::with_path(old_path.clone());
    old_storage
        .store_github_token("ghp_old_xor_token")
        .unwrap();
    assert!(old_path.exists());

    // Create NativeStorage — should trigger migration
    let storage = NativeStorage::new(new_path.clone(), "default".to_string()).unwrap();

    // Old file should be gone (security: remove insecure XOR storage)
    assert!(
        !old_path.exists(),
        "Old credentials.json should be deleted after migration"
    );

    // Token should be retrievable via new format
    let token = storage.get_github_token().unwrap();
    assert_eq!(token, "ghp_old_xor_token");

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn native_storage_constructor_with_profiles() {
    let dir = test_dir("profiles");
    let path = dir.join("github-copilot.json");

    let storage = NativeStorage::new(path, "my-profile".to_string()).unwrap();
    // Just verifying construction doesn't fail with profile names
    drop(storage);

    let _ = fs::remove_dir_all(&dir);
}
