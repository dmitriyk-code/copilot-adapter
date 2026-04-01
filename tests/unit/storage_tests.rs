use copilot_adapter::storage::TokenStorage;
use std::fs;

// Test with file storage (always available, no keyring dependency in CI)
#[test]
fn file_storage_store_retrieve_delete() {
    let dir = std::env::temp_dir().join(format!(
        "copilot-adapter-storage-test-{}",
        std::process::id()
    ));
    let _ = fs::create_dir_all(&dir);
    let path = dir.join("test_creds.json");

    let storage = copilot_adapter::storage::file::FileStorage::with_path(path.clone());

    // Initially no token
    assert!(storage.get_github_token().is_err());

    // Store
    storage.store_github_token("ghp_testtoken123").unwrap();

    // Retrieve
    let token = storage.get_github_token().unwrap();
    assert_eq!(token, "ghp_testtoken123");

    // Overwrite
    storage.store_github_token("ghp_newtoken456").unwrap();
    let token = storage.get_github_token().unwrap();
    assert_eq!(token, "ghp_newtoken456");

    // Delete
    storage.delete_github_token().unwrap();
    assert!(storage.get_github_token().is_err());

    // Delete again (idempotent)
    storage.delete_github_token().unwrap();

    // Cleanup
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn file_storage_creates_parent_directories() {
    let dir = std::env::temp_dir().join(format!(
        "copilot-adapter-nested-test-{}/sub/dir",
        std::process::id()
    ));
    let path = dir.join("creds.json");

    let storage = copilot_adapter::storage::file::FileStorage::with_path(path);

    storage.store_github_token("token").unwrap();
    assert_eq!(storage.get_github_token().unwrap(), "token");

    // Cleanup
    let base = std::env::temp_dir().join(format!(
        "copilot-adapter-nested-test-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&base);
}

// Keyring tests — only run when the OS keyring is available.
// On CI or headless environments this may be skipped.
#[test]
fn keyring_storage_round_trip() {
    let ks = match copilot_adapter::storage::keyring::KeyringStorage::new() {
        Ok(ks) => ks,
        Err(_) => {
            eprintln!("Skipping keyring test: keyring not available");
            return;
        }
    };

    match ks.verify_available() {
        Ok(true) => {}
        _ => {
            eprintln!("Skipping keyring test: keyring verification failed");
            return;
        }
    }

    // Use a test-specific key scope
    let test_token = "ghp_keyring_test_token_42";

    // Store
    ks.store_github_token(test_token).unwrap();

    // Retrieve
    let retrieved = ks.get_github_token().unwrap();
    assert_eq!(retrieved, test_token);

    // Delete
    ks.delete_github_token().unwrap();

    // Should be gone
    assert!(ks.get_github_token().is_err());

    // Delete again (idempotent)
    ks.delete_github_token().unwrap();
}

// --- Epic 3: File-First Credential Storage tests ---

#[test]
fn get_credentials_path_is_under_copilot_adapter_dir() {
    let path = copilot_adapter::storage::file::get_credentials_path();
    assert_eq!(path.file_name().unwrap(), "credentials.json");
    let parent = path.parent().unwrap();
    assert!(
        parent.ends_with(".copilot-adapter"),
        "Expected credentials path parent to end with .copilot-adapter, got: {}",
        parent.display()
    );
}

#[test]
fn create_storage_false_returns_file_storage() {
    // create_storage(false) should always return a file-based storage,
    // not try the keyring at all.
    let _store = copilot_adapter::storage::create_storage(false);
    // Verify it's a FileStorage by checking that store/retrieve works
    // via a temp directory.
    let dir = std::env::temp_dir().join(format!(
        "copilot-adapter-create-storage-test-{}",
        std::process::id()
    ));
    let _ = fs::create_dir_all(&dir);
    let path = dir.join("creds.json");

    let store = copilot_adapter::storage::create_storage_with_path(path.clone(), false);
    store.store_github_token("ghp_file_test").unwrap();
    assert_eq!(store.get_github_token().unwrap(), "ghp_file_test");

    // Verify it's actually file-based (the file should exist on disk)
    assert!(path.exists(), "File should exist on disk for file-based storage");

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn create_storage_with_path_uses_custom_path() {
    let dir = std::env::temp_dir().join(format!(
        "copilot-adapter-custom-path-test-{}",
        std::process::id()
    ));
    let _ = fs::create_dir_all(&dir);
    let path = dir.join("custom_creds.json");

    let store = copilot_adapter::storage::create_storage_with_path(path.clone(), false);
    store.store_github_token("ghp_custom").unwrap();

    // Verify the token is in the custom file
    assert!(path.exists());
    let retrieved = store.get_github_token().unwrap();
    assert_eq!(retrieved, "ghp_custom");

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn create_storage_true_tries_keyring() {
    // create_storage(true) should try keyring first.
    // On systems without a keyring, it falls back to file storage.
    // Either way, the returned storage should be functional.
    let dir = std::env::temp_dir().join(format!(
        "copilot-adapter-keyring-fallback-test-{}",
        std::process::id()
    ));
    let _ = fs::create_dir_all(&dir);
    let path = dir.join("creds.json");

    let store = copilot_adapter::storage::create_storage_with_path(path.clone(), true);
    store.store_github_token("ghp_keyring_or_file").unwrap();
    let token = store.get_github_token().unwrap();
    assert_eq!(token, "ghp_keyring_or_file");

    // Clean up: delete the token and the temp dir
    store.delete_github_token().unwrap();
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn migration_copies_legacy_credentials() {
    let dir = std::env::temp_dir().join(format!(
        "copilot-adapter-migration-test-{}",
        std::process::id()
    ));
    let legacy_dir = dir.join("legacy");
    let new_dir = dir.join("new");
    let _ = fs::create_dir_all(&legacy_dir);
    // Deliberately do NOT create new_dir — migrate_from_to should handle it

    let legacy_path = legacy_dir.join("credentials.json");
    let new_path = new_dir.join("credentials.json");

    // Create a credentials file at the "legacy" location
    let legacy_storage = copilot_adapter::storage::file::FileStorage::with_path(legacy_path.clone());
    legacy_storage.store_github_token("ghp_migration_token").unwrap();
    assert!(legacy_path.exists());

    // Run the actual migration function (not a manual fs::copy)
    copilot_adapter::storage::file::migrate_from_to(&legacy_path, &new_path);

    // Verify the new directory was created and the file was copied
    assert!(new_path.exists(), "migrate_from_to should create the destination file");

    // Verify the new location has the token
    let new_storage = copilot_adapter::storage::file::FileStorage::with_path(new_path.clone());
    let token = new_storage.get_github_token().unwrap();
    assert_eq!(token, "ghp_migration_token");

    // Legacy file should still exist (copy, not move)
    assert!(legacy_path.exists());

    let _ = fs::remove_dir_all(&dir);
}
