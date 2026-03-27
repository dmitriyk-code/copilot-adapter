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
