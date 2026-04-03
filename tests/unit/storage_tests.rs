use copilot_adapter::storage::TokenStorage;
use std::fs;

// --- Epic 4: Native Storage factory tests ---

#[test]
fn create_storage_for_profile_returns_working_storage() {
    let dir = std::env::temp_dir().join(format!(
        "copilot-adapter-factory-test-{}",
        std::process::id()
    ));
    let _ = fs::create_dir_all(&dir);
    let path = dir.join("github-copilot.json");

    let store = copilot_adapter::storage::create_storage_for_profile(
        path.clone(),
        "test-profile".to_string(),
    )
    .unwrap();
    store.store_github_token("ghp_factory_test").unwrap();
    assert_eq!(store.get_github_token().unwrap(), "ghp_factory_test");

    // Verify credential file exists at the expected path
    assert!(path.exists(), "Credential file should exist on disk");

    store.delete_github_token().unwrap();
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn create_storage_for_profile_creates_parent_directories() {
    let dir = std::env::temp_dir().join(format!(
        "copilot-adapter-factory-nested-{}/deep/path",
        std::process::id()
    ));
    let path = dir.join("github-copilot.json");

    let store = copilot_adapter::storage::create_storage_for_profile(
        path.clone(),
        "nested-profile".to_string(),
    )
    .unwrap();
    store.store_github_token("ghp_nested").unwrap();
    assert_eq!(store.get_github_token().unwrap(), "ghp_nested");

    store.delete_github_token().unwrap();
    let base = std::env::temp_dir().join(format!(
        "copilot-adapter-factory-nested-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&base);
}
