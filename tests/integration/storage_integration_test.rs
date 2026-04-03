//! Integration tests for the native credential storage system.
//!
//! These tests exercise NativeStorage through the public API, simulating
//! real-world scenarios: credential persistence across restarts, profile
//! isolation, and logout cleanup.

use copilot_adapter::storage::native::NativeStorage;
use copilot_adapter::storage::TokenStorage;
use std::fs;

/// Helper to create a unique temp directory for each test.
fn test_dir(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "copilot-adapter-storage-integ-{}-{}",
        name,
        std::process::id()
    ));
    // Clean up leftovers from a prior run
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::create_dir_all(&dir);
    dir
}

// ============================================================
// Scenario 1: Auth flow with credential persistence
// ============================================================

#[cfg(target_os = "windows")]
#[test]
fn credential_persistence_across_instances() {
    let dir = test_dir("persist");
    let path = dir.join("github-copilot.json");

    // First instance stores a token (simulates initial `copilot-adapter auth`)
    {
        let storage = NativeStorage::new(path.clone(), "default".to_string()).unwrap();
        storage.store_github_token("ghp_persist_test_token").unwrap();
    }
    // storage dropped — simulates adapter shutdown

    // Second instance reads it back (simulates restart)
    {
        let storage = NativeStorage::new(path.clone(), "default".to_string()).unwrap();
        let token = storage.get_github_token().unwrap();
        assert_eq!(token, "ghp_persist_test_token");
    }

    let _ = fs::remove_dir_all(&dir);
}

#[cfg(target_os = "windows")]
#[test]
fn credential_update_persists() {
    let dir = test_dir("update-persist");
    let path = dir.join("github-copilot.json");

    // Store initial token
    {
        let storage = NativeStorage::new(path.clone(), "default".to_string()).unwrap();
        storage.store_github_token("ghp_original").unwrap();
    }

    // Update token (simulates `copilot-adapter auth --force`)
    {
        let storage = NativeStorage::new(path.clone(), "default".to_string()).unwrap();
        storage.store_github_token("ghp_refreshed").unwrap();
    }

    // Read back — should see updated token
    {
        let storage = NativeStorage::new(path.clone(), "default".to_string()).unwrap();
        let token = storage.get_github_token().unwrap();
        assert_eq!(token, "ghp_refreshed");
    }

    let _ = fs::remove_dir_all(&dir);
}

// ============================================================
// Scenario 2: Profile isolation
// ============================================================

#[cfg(target_os = "windows")]
#[test]
fn profiles_are_isolated() {
    let base = test_dir("profile-isolation");

    // Create two profile directories (simulates ~/.copilot-adapter/profiles/<name>/)
    let dir_a = base.join("profiles").join("profile_a");
    let dir_b = base.join("profiles").join("profile_b");
    fs::create_dir_all(&dir_a).unwrap();
    fs::create_dir_all(&dir_b).unwrap();

    let path_a = dir_a.join("github-copilot.json");
    let path_b = dir_b.join("github-copilot.json");

    // Store different tokens in each profile
    let storage_a = NativeStorage::new(path_a.clone(), "profile_a".to_string()).unwrap();
    let storage_b = NativeStorage::new(path_b.clone(), "profile_b".to_string()).unwrap();

    storage_a.store_github_token("ghp_alpha_token").unwrap();
    storage_b.store_github_token("ghp_beta_token").unwrap();

    // Each profile should retrieve only its own token
    assert_eq!(storage_a.get_github_token().unwrap(), "ghp_alpha_token");
    assert_eq!(storage_b.get_github_token().unwrap(), "ghp_beta_token");

    // Verify no cross-contamination after re-instantiation
    let storage_a2 = NativeStorage::new(path_a, "profile_a".to_string()).unwrap();
    let storage_b2 = NativeStorage::new(path_b, "profile_b".to_string()).unwrap();

    assert_eq!(storage_a2.get_github_token().unwrap(), "ghp_alpha_token");
    assert_eq!(storage_b2.get_github_token().unwrap(), "ghp_beta_token");

    let _ = fs::remove_dir_all(&base);
}

#[cfg(target_os = "windows")]
#[test]
fn deleting_one_profile_does_not_affect_other() {
    let base = test_dir("profile-delete-isolation");

    let dir_a = base.join("profiles").join("work");
    let dir_b = base.join("profiles").join("personal");
    fs::create_dir_all(&dir_a).unwrap();
    fs::create_dir_all(&dir_b).unwrap();

    let path_a = dir_a.join("github-copilot.json");
    let path_b = dir_b.join("github-copilot.json");

    let storage_a = NativeStorage::new(path_a.clone(), "work".to_string()).unwrap();
    let storage_b = NativeStorage::new(path_b.clone(), "personal".to_string()).unwrap();

    storage_a.store_github_token("ghp_work").unwrap();
    storage_b.store_github_token("ghp_personal").unwrap();

    // Delete profile A's credentials
    storage_a.delete_github_token().unwrap();

    // Profile A should have no credentials
    assert!(!path_a.exists());
    assert!(storage_a.get_github_token().is_err());

    // Profile B should be unaffected
    assert_eq!(storage_b.get_github_token().unwrap(), "ghp_personal");

    let _ = fs::remove_dir_all(&base);
}

// ============================================================
// Scenario 3: Logout cleanup
// ============================================================

#[cfg(target_os = "windows")]
#[test]
fn logout_removes_credential_file() {
    let dir = test_dir("logout");
    let path = dir.join("github-copilot.json");

    let storage = NativeStorage::new(path.clone(), "default".to_string()).unwrap();
    storage.store_github_token("ghp_to_be_logged_out").unwrap();
    assert!(path.exists());

    // Simulate `copilot-adapter logout`
    storage.delete_github_token().unwrap();

    // Credential file should be removed
    assert!(
        !path.exists(),
        "Credential file should be removed after logout"
    );

    // Subsequent get should fail
    let err = storage.get_github_token().unwrap_err();
    assert!(
        err.to_string().contains("No credentials found"),
        "Expected 'No credentials found' after logout, got: {}",
        err
    );

    let _ = fs::remove_dir_all(&dir);
}

#[cfg(target_os = "windows")]
#[test]
fn logout_then_reauth_works() {
    let dir = test_dir("logout-reauth");
    let path = dir.join("github-copilot.json");

    let storage = NativeStorage::new(path.clone(), "default".to_string()).unwrap();

    // First auth
    storage.store_github_token("ghp_first_session").unwrap();
    assert_eq!(storage.get_github_token().unwrap(), "ghp_first_session");

    // Logout
    storage.delete_github_token().unwrap();
    assert!(storage.get_github_token().is_err());

    // Re-auth with new token
    storage.store_github_token("ghp_second_session").unwrap();
    assert_eq!(storage.get_github_token().unwrap(), "ghp_second_session");

    let _ = fs::remove_dir_all(&dir);
}

// ============================================================
// Scenario: create_storage_for_profile factory function
// ============================================================

#[cfg(target_os = "windows")]
#[test]
fn factory_function_produces_working_storage() {
    let dir = test_dir("factory");
    let path = dir.join("github-copilot.json");

    let storage = copilot_adapter::storage::create_storage_for_profile(
        path.clone(),
        "integration-test".to_string(),
    )
    .unwrap();

    storage.store_github_token("ghp_factory_integ").unwrap();

    // Re-create via factory to simulate restart
    let storage2 = copilot_adapter::storage::create_storage_for_profile(
        path,
        "integration-test".to_string(),
    )
    .unwrap();

    assert_eq!(storage2.get_github_token().unwrap(), "ghp_factory_integ");

    storage2.delete_github_token().unwrap();
    let _ = fs::remove_dir_all(&dir);
}
