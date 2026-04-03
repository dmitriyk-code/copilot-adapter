//! Integration tests for profile lifecycle, multi-instance management,
//! and migration from flat directory layout to profile-based layout.
//!
//! These tests exercise ProfileManager, status file management, credential
//! storage, and migration logic across profiles. They use isolated temp
//! directories to avoid interfering with each other or with real user data.

use copilot_adapter::daemon::status::{
    read_status_from, remove_status_from, write_status_to, StatusFile,
};
use copilot_adapter::profile::migration::run_migration;
use copilot_adapter::profile::ProfileManager;
use copilot_adapter::storage::TokenStorage;
use std::fs;

/// Helper: create a unique temp directory for each test to avoid interference.
fn test_dir(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "copilot-adapter-integ-profile-{}-{}",
        name,
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::create_dir_all(&dir);
    dir
}

// ============================================================
// Task 8.4: Profile lifecycle test
// ============================================================

#[test]
fn profile_full_lifecycle_create_auth_status_stop_delete() {
    let base = test_dir("lifecycle");
    let mgr = ProfileManager::with_base_dir(base.clone());

    // Step 1: Create a profile
    let profile = mgr.create("staging").unwrap();
    assert_eq!(profile.name, "staging");
    assert!(profile.dir.exists());
    assert!(profile.dir.is_dir());

    // Step 2: Simulate auth — store credentials
    let storage = copilot_adapter::storage::create_storage_for_profile(
        profile.credentials_path(),
        profile.name.clone(),
    )
    .unwrap();
    storage.store_github_token("ghp_staging_token").unwrap();
    assert!(profile.credentials_path().exists());

    // Step 3: Simulate start — write status file
    write_status_to(&profile.status_path(), 8080).unwrap();
    assert!(profile.status_path().exists());

    // Step 4: Check status
    let status = read_status_from(&profile.status_path()).unwrap();
    assert_eq!(status.pid, std::process::id());
    assert_eq!(status.port, 8080);

    // Step 5: Simulate stop — remove status file
    remove_status_from(&profile.status_path());
    assert!(!profile.status_path().exists());
    assert!(
        read_status_from(&profile.status_path()).is_none(),
        "status should be None after stop"
    );

    // Step 6: Clean up keyring entry before deleting profile
    storage.delete_github_token().unwrap_or_default();

    // Step 7: Delete the profile
    mgr.delete("staging").unwrap();
    assert!(!profile.dir.exists(), "profile directory should be removed");

    let _ = fs::remove_dir_all(&base);
}

#[test]
fn profile_lifecycle_default_auto_creates() {
    let base = test_dir("default-lifecycle");
    let mgr = ProfileManager::with_base_dir(base.clone());

    // Default profile auto-creates on get()
    let profile = mgr.get("default").unwrap();
    assert_eq!(profile.name, "default");
    assert!(profile.dir.exists());

    // Store credentials and status
    let storage = copilot_adapter::storage::create_storage_for_profile(
        profile.credentials_path(),
        profile.name.clone(),
    )
    .unwrap();
    storage.store_github_token("ghp_default_token").unwrap();
    write_status_to(&profile.status_path(), 6767).unwrap();

    // Verify both exist
    assert!(profile.credentials_path().exists());
    assert!(profile.status_path().exists());

    // Cannot delete default
    let result = mgr.delete("default");
    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("Cannot delete"));

    // Clean up
    storage.delete_github_token().unwrap_or_default();
    remove_status_from(&profile.status_path());
    let _ = fs::remove_dir_all(&base);
}

#[test]
fn profile_list_reflects_created_profiles() {
    let base = test_dir("list-lifecycle");
    let mgr = ProfileManager::with_base_dir(base.clone());

    // Initially empty
    assert!(mgr.list().is_empty());

    // Create default (via get) and two named profiles
    mgr.get("default").unwrap();
    mgr.create("work").unwrap();
    mgr.create("personal").unwrap();

    let mut names: Vec<String> = mgr.list().into_iter().map(|p| p.name).collect();
    names.sort();
    assert_eq!(names, vec!["default", "personal", "work"]);

    // Delete one
    mgr.delete("personal").unwrap();
    let mut names: Vec<String> = mgr.list().into_iter().map(|p| p.name).collect();
    names.sort();
    assert_eq!(names, vec!["default", "work"]);

    let _ = fs::remove_dir_all(&base);
}

#[test]
fn profile_get_nonexistent_fails() {
    let base = test_dir("get-nonexistent");
    let mgr = ProfileManager::with_base_dir(base.clone());

    let result = mgr.get("nonexistent");
    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("does not exist"));

    let _ = fs::remove_dir_all(&base);
}

#[test]
fn profile_create_duplicate_fails() {
    let base = test_dir("create-dup");
    let mgr = ProfileManager::with_base_dir(base.clone());

    mgr.create("dup").unwrap();
    let result = mgr.create("dup");
    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("already exists"));

    let _ = fs::remove_dir_all(&base);
}

#[test]
fn profile_create_default_fails() {
    let base = test_dir("create-default-fail");
    let mgr = ProfileManager::with_base_dir(base.clone());

    let result = mgr.create("default");
    assert!(result.is_err());

    let _ = fs::remove_dir_all(&base);
}

#[test]
fn profile_name_validation_integration() {
    let base = test_dir("name-validation");
    let mgr = ProfileManager::with_base_dir(base.clone());

    // Valid names
    assert!(mgr.create("valid-name").is_ok());
    assert!(mgr.create("with_underscore").is_ok());
    assert!(mgr.create("abc123").is_ok());

    // Invalid names
    assert!(mgr.create("has space").is_err());
    assert!(mgr.create("has.dot").is_err());
    assert!(mgr.create("").is_err());
    assert!(mgr.create(&"x".repeat(65)).is_err());

    let _ = fs::remove_dir_all(&base);
}

// ============================================================
// Task 8.5: Multi-instance test
// ============================================================

#[test]
fn multi_instance_two_profiles_different_ports() {
    let base = test_dir("multi-instance");
    let mgr = ProfileManager::with_base_dir(base.clone());

    // Create two profiles
    let default = mgr.get("default").unwrap();
    let work = mgr.create("work").unwrap();

    // Simulate both running on different ports
    write_status_to(&default.status_path(), 6767).unwrap();
    write_status_to(&work.status_path(), 8080).unwrap();

    // Both should have status files
    let s1 = read_status_from(&default.status_path()).unwrap();
    let s2 = read_status_from(&work.status_path()).unwrap();
    assert_eq!(s1.port, 6767);
    assert_eq!(s2.port, 8080);

    // Both are running (same process PID for testing purposes)
    assert_eq!(s1.pid, std::process::id());
    assert_eq!(s2.pid, std::process::id());

    // Store separate credentials
    let default_storage = copilot_adapter::storage::create_storage_for_profile(
        default.credentials_path(),
        default.name.clone(),
    )
    .unwrap();
    let work_storage = copilot_adapter::storage::create_storage_for_profile(
        work.credentials_path(),
        work.name.clone(),
    )
    .unwrap();
    default_storage.store_github_token("ghp_default").unwrap();
    work_storage.store_github_token("ghp_work").unwrap();

    // Verify credential isolation
    assert_eq!(default_storage.get_github_token().unwrap(), "ghp_default");
    assert_eq!(work_storage.get_github_token().unwrap(), "ghp_work");

    // Find by port should locate the right profile
    let found_6767 = mgr.find_by_port(6767);
    assert!(found_6767.is_some());
    assert_eq!(found_6767.unwrap().name, "default");

    let found_8080 = mgr.find_by_port(8080);
    assert!(found_8080.is_some());
    assert_eq!(found_8080.unwrap().name, "work");

    // Port not in use returns None
    assert!(mgr.find_by_port(9999).is_none());

    // Clean up keyring entries
    default_storage.delete_github_token().unwrap_or_default();
    work_storage.delete_github_token().unwrap_or_default();
    remove_status_from(&default.status_path());
    remove_status_from(&work.status_path());
    let _ = fs::remove_dir_all(&base);
}

#[test]
fn multi_instance_port_conflict_rejection() {
    let base = test_dir("port-conflict");
    let mgr = ProfileManager::with_base_dir(base.clone());

    // Default running on port 6767
    let default = mgr.get("default").unwrap();
    write_status_to(&default.status_path(), 6767).unwrap();

    // Create work profile
    let _work = mgr.create("work").unwrap();

    // Attempting to use port 6767 for "work" should be rejected
    let result = mgr.check_port_conflict(6767, "work");
    assert!(result.is_err());
    let err = result.unwrap_err().to_string();
    assert!(err.contains("6767"), "error should mention the port");
    assert!(
        err.contains("default"),
        "error should mention the conflicting profile"
    );

    // Same port for same profile should be OK
    assert!(mgr.check_port_conflict(6767, "default").is_ok());

    // Different port for "work" should be OK
    assert!(mgr.check_port_conflict(8080, "work").is_ok());

    // Clean up
    remove_status_from(&default.status_path());
    let _ = fs::remove_dir_all(&base);
}

#[test]
fn multi_instance_stale_port_conflict_is_ignored() {
    let base = test_dir("stale-conflict");
    let mgr = ProfileManager::with_base_dir(base.clone());

    // Create "default" with stale status (dead PID)
    let default = mgr.get("default").unwrap();
    let stale_status = StatusFile {
        pid: 99999999, // bogus PID — not a real process
        port: 6767,
        started_at: None,
        version: None,
    };
    fs::write(
        &default.status_path(),
        serde_json::to_string_pretty(&stale_status).unwrap(),
    )
    .unwrap();

    // Create "work" profile
    let _work = mgr.create("work").unwrap();

    // Port conflict check should pass because the default process is dead
    assert!(
        mgr.check_port_conflict(6767, "work").is_ok(),
        "stale status should not cause port conflict"
    );

    // Stale status file should have been cleaned up
    assert!(
        !default.status_path().exists(),
        "stale status file should be removed during conflict check"
    );

    let _ = fs::remove_dir_all(&base);
}

#[test]
fn multi_instance_all_operations() {
    let base = test_dir("all-ops");
    let mgr = ProfileManager::with_base_dir(base.clone());

    // Create multiple profiles
    let default = mgr.get("default").unwrap();
    let work = mgr.create("work").unwrap();
    let staging = mgr.create("staging").unwrap();

    // Start all on different ports
    write_status_to(&default.status_path(), 6767).unwrap();
    write_status_to(&work.status_path(), 7070).unwrap();
    write_status_to(&staging.status_path(), 8080).unwrap();

    // "Status --all": list all and check status
    let profiles = mgr.list();
    assert_eq!(profiles.len(), 3);

    for profile in &profiles {
        let status = read_status_from(&profile.status_path());
        assert!(
            status.is_some(),
            "profile '{}' should have status",
            profile.name
        );
    }

    // "Stop --all": remove all status files
    for profile in &profiles {
        remove_status_from(&profile.status_path());
    }

    // Verify all stopped
    for profile in &profiles {
        assert!(
            read_status_from(&profile.status_path()).is_none(),
            "profile '{}' should have no status after stop --all",
            profile.name
        );
    }

    let _ = fs::remove_dir_all(&base);
}

#[test]
fn multi_instance_find_by_port_cleans_stale_and_finds_live() {
    let base = test_dir("find-stale-live");
    let mgr = ProfileManager::with_base_dir(base.clone());

    // "stale" profile with dead PID on port 8080
    let stale_profile = mgr.create("stale").unwrap();
    let stale_status = StatusFile {
        pid: 99999999,
        port: 8080,
        started_at: None,
        version: None,
    };
    fs::write(
        &stale_profile.status_path(),
        serde_json::to_string_pretty(&stale_status).unwrap(),
    )
    .unwrap();

    // "live" profile with current PID on port 9090
    let live_profile = mgr.create("live").unwrap();
    write_status_to(&live_profile.status_path(), 9090).unwrap();

    // find_by_port(8080) should return None (stale, cleaned up)
    assert!(mgr.find_by_port(8080).is_none());
    assert!(
        !stale_profile.status_path().exists(),
        "stale status should be cleaned up"
    );

    // find_by_port(9090) should return the live profile
    let found = mgr.find_by_port(9090);
    assert!(found.is_some());
    assert_eq!(found.unwrap().name, "live");

    let _ = fs::remove_dir_all(&base);
}

// ============================================================
// Task 8.6: Migration test
// ============================================================

#[test]
fn migration_flat_status_to_default_profile() {
    let base = test_dir("migrate-status");

    // Create flat-dir status.json (pre-profile layout)
    let status = StatusFile {
        pid: std::process::id(),
        port: 6767,
        started_at: Some("2026-04-01T12:00:00+00:00".to_string()),
        version: Some("0.1.0".to_string()),
    };
    fs::write(
        base.join("status.json"),
        serde_json::to_string_pretty(&status).unwrap(),
    )
    .unwrap();

    // Run migration
    run_migration(&base, None);

    // Flat file should be gone
    assert!(
        !base.join("status.json").exists(),
        "flat status.json should be removed"
    );

    // Profile file should exist
    let profile_path = base.join("profiles").join("default").join("status.json");
    assert!(profile_path.exists(), "status.json should be in profiles/default/");

    // Content should be preserved
    let migrated = read_status_from(&profile_path).unwrap();
    assert_eq!(migrated.pid, std::process::id());
    assert_eq!(migrated.port, 6767);

    let _ = fs::remove_dir_all(&base);
}

#[test]
fn migration_flat_credentials_to_default_profile() {
    let base = test_dir("migrate-creds");

    // Create flat-dir credentials as raw bytes (profile migration just moves files)
    let cred_data = b"test-credential-data";
    fs::write(base.join("credentials.json"), cred_data).unwrap();

    // Run migration
    run_migration(&base, None);

    // Flat file should be gone
    assert!(!base.join("credentials.json").exists());

    // Profile file should exist with identical content
    let profile_path = base
        .join("profiles")
        .join("default")
        .join("credentials.json");
    assert!(profile_path.exists());
    assert_eq!(fs::read(&profile_path).unwrap(), cred_data);

    let _ = fs::remove_dir_all(&base);
}

#[test]
fn migration_both_files_to_default_profile() {
    let base = test_dir("migrate-both");

    // Create flat-dir status and credentials
    fs::write(
        base.join("status.json"),
        r#"{"pid":12345,"port":6767}"#,
    )
    .unwrap();
    fs::write(base.join("credentials.json"), b"cred-data").unwrap();

    run_migration(&base, None);

    // Both flat files should be gone
    assert!(!base.join("status.json").exists());
    assert!(!base.join("credentials.json").exists());

    // Both should be in profiles/default/
    let default_dir = base.join("profiles").join("default");
    assert!(default_dir.join("status.json").exists());
    assert!(default_dir.join("credentials.json").exists());

    let _ = fs::remove_dir_all(&base);
}

#[test]
fn migration_is_noop_when_profiles_dir_exists() {
    let base = test_dir("migrate-noop");

    // Create profiles/ dir (simulates already migrated)
    fs::create_dir_all(base.join("profiles").join("default")).unwrap();

    // Create flat files that should NOT be moved
    fs::write(base.join("status.json"), r#"{"pid":1,"port":1}"#).unwrap();
    fs::write(base.join("credentials.json"), b"creds").unwrap();

    run_migration(&base, None);

    // Flat files should still exist (not moved)
    assert!(base.join("status.json").exists());
    assert!(base.join("credentials.json").exists());

    let _ = fs::remove_dir_all(&base);
}

#[test]
fn migration_is_idempotent() {
    let base = test_dir("migrate-idempotent");

    fs::write(
        base.join("status.json"),
        r#"{"pid":1,"port":6767}"#,
    )
    .unwrap();
    fs::write(base.join("credentials.json"), b"creds-data").unwrap();

    // First migration
    run_migration(&base, None);

    let default_dir = base.join("profiles").join("default");
    let status_content = fs::read_to_string(default_dir.join("status.json")).unwrap();
    let creds_content = fs::read(default_dir.join("credentials.json")).unwrap();

    // Second migration — should be a no-op
    run_migration(&base, None);

    // Content should be unchanged
    assert_eq!(
        fs::read_to_string(default_dir.join("status.json")).unwrap(),
        status_content
    );
    assert_eq!(
        fs::read(default_dir.join("credentials.json")).unwrap(),
        creds_content
    );

    let _ = fs::remove_dir_all(&base);
}

#[test]
fn migration_legacy_pid_running_process_synthesizes_status() {
    let base = test_dir("legacy-pid-running");
    let legacy_dir = std::env::temp_dir().join(format!(
        "copilot-adapter-integ-legacy-running-{}",
        std::process::id()
    ));
    let _ = fs::create_dir_all(&legacy_dir);
    let pid_path = legacy_dir.join("copilot-adapter.pid");
    let port_path = legacy_dir.join("copilot-adapter.port");

    // Write current process PID (definitely running)
    fs::write(&pid_path, std::process::id().to_string()).unwrap();
    fs::write(&port_path, "9090").unwrap();

    run_migration(&base, Some(&pid_path));

    // Legacy files should be cleaned up
    assert!(!pid_path.exists(), "legacy PID file should be removed");
    assert!(!port_path.exists(), "legacy port file should be removed");

    // A synthesized status.json should exist in default profile
    let status_path = base.join("profiles").join("default").join("status.json");
    assert!(status_path.exists());

    let status = read_status_from(&status_path).unwrap();
    assert_eq!(status.pid, std::process::id());
    assert_eq!(status.port, 9090);
    // Synthesized from legacy — no started_at or version
    assert!(status.started_at.is_none());
    assert!(status.version.is_none());

    let _ = fs::remove_dir_all(&base);
    let _ = fs::remove_dir_all(&legacy_dir);
}

#[test]
fn migration_legacy_pid_dead_process_cleans_up() {
    let base = test_dir("legacy-pid-dead");
    let legacy_dir = std::env::temp_dir().join(format!(
        "copilot-adapter-integ-legacy-dead-{}",
        std::process::id()
    ));
    let _ = fs::create_dir_all(&legacy_dir);
    let pid_path = legacy_dir.join("copilot-adapter.pid");
    let port_path = legacy_dir.join("copilot-adapter.port");

    // Write a PID for a dead process
    fs::write(&pid_path, "99999999").unwrap();
    fs::write(&port_path, "7070").unwrap();

    run_migration(&base, Some(&pid_path));

    // Legacy files should be cleaned up
    assert!(!pid_path.exists());
    assert!(!port_path.exists());

    // No status.json for dead process
    let status_path = base.join("profiles").join("default").join("status.json");
    assert!(
        !status_path.exists(),
        "should not create status.json for dead process"
    );

    let _ = fs::remove_dir_all(&base);
    let _ = fs::remove_dir_all(&legacy_dir);
}

#[test]
fn migration_legacy_pid_with_flat_files_preserves_flat_data() {
    let base = test_dir("legacy-with-flat");
    let legacy_dir = std::env::temp_dir().join(format!(
        "copilot-adapter-integ-legacy-flat-{}",
        std::process::id()
    ));
    let _ = fs::create_dir_all(&legacy_dir);
    let pid_path = legacy_dir.join("copilot-adapter.pid");
    let port_path = legacy_dir.join("copilot-adapter.port");

    // Flat-dir files (will be migrated first, taking precedence)
    let original_status = r#"{"pid":42,"port":6767,"started_at":"2026-04-01T00:00:00+00:00","version":"1.0.0"}"#;
    fs::write(base.join("status.json"), original_status).unwrap();
    fs::write(base.join("credentials.json"), b"flat-creds").unwrap();

    // Legacy PID file (current process, running)
    fs::write(&pid_path, std::process::id().to_string()).unwrap();
    fs::write(&port_path, "9999").unwrap();

    run_migration(&base, Some(&pid_path));

    // All source files should be cleaned up
    assert!(!base.join("status.json").exists());
    assert!(!base.join("credentials.json").exists());
    assert!(!pid_path.exists());
    assert!(!port_path.exists());

    // Status should contain flat-dir data (not synthesized from PID)
    let default_dir = base.join("profiles").join("default");
    let status = read_status_from(&default_dir.join("status.json")).unwrap();
    assert_eq!(
        status.pid, 42,
        "should keep flat-dir status, not legacy PID"
    );
    assert_eq!(
        status.port, 6767,
        "should keep flat-dir port, not legacy port"
    );

    // Credentials should be intact
    let creds = fs::read(default_dir.join("credentials.json")).unwrap();
    assert_eq!(creds, b"flat-creds");

    let _ = fs::remove_dir_all(&base);
    let _ = fs::remove_dir_all(&legacy_dir);
}

#[test]
fn migration_noop_when_no_data() {
    let base = test_dir("migrate-no-data");

    // Empty base dir — nothing to migrate
    run_migration(&base, None);

    // profiles/ should NOT have been created
    assert!(!base.join("profiles").exists());

    let _ = fs::remove_dir_all(&base);
}

#[test]
fn migration_post_migrate_profile_manager_works() {
    let base = test_dir("post-migrate-pm");

    // Create flat-dir data
    fs::write(base.join("credentials.json"), b"migrated-creds").unwrap();

    // Migrate
    run_migration(&base, None);

    // After migration, ProfileManager should work correctly
    let mgr = ProfileManager::with_base_dir(base.clone());
    let default = mgr.get("default").unwrap();
    assert_eq!(default.name, "default");

    // Migrated credentials should be accessible at the old filename.
    // NativeStorage will auto-migrate from credentials.json → github-copilot.json
    // on first access, but the profile migration itself preserves the original name.
    assert!(
        default.dir.join("credentials.json").exists(),
        "profile migration should have copied credentials.json into the profile directory"
    );

    // Can create additional profiles
    let work = mgr.create("work").unwrap();
    assert!(work.dir.exists());

    // List should include both
    let mut names: Vec<String> = mgr.list().into_iter().map(|p| p.name).collect();
    names.sort();
    assert_eq!(names, vec!["default", "work"]);

    let _ = fs::remove_dir_all(&base);
}

#[test]
fn migration_legacy_pid_without_port_file_uses_zero() {
    let base = test_dir("legacy-no-port");
    let legacy_dir = std::env::temp_dir().join(format!(
        "copilot-adapter-integ-legacy-noport-{}",
        std::process::id()
    ));
    let _ = fs::create_dir_all(&legacy_dir);
    let pid_path = legacy_dir.join("copilot-adapter.pid");

    // Write current process PID but no port file
    fs::write(&pid_path, std::process::id().to_string()).unwrap();

    run_migration(&base, Some(&pid_path));

    let status_path = base.join("profiles").join("default").join("status.json");
    let status = read_status_from(&status_path).unwrap();
    assert_eq!(status.pid, std::process::id());
    assert_eq!(status.port, 0, "port should default to 0 when port file is missing");

    let _ = fs::remove_dir_all(&base);
    let _ = fs::remove_dir_all(&legacy_dir);
}

#[test]
fn migration_legacy_pid_invalid_content_cleans_up() {
    let base = test_dir("legacy-invalid");
    let legacy_dir = std::env::temp_dir().join(format!(
        "copilot-adapter-integ-legacy-invalid-{}",
        std::process::id()
    ));
    let _ = fs::create_dir_all(&legacy_dir);
    let pid_path = legacy_dir.join("copilot-adapter.pid");
    let port_path = pid_path.with_extension("port");

    // Write invalid PID content
    fs::write(&pid_path, "not-a-number").unwrap();
    fs::write(&port_path, "7070").unwrap();

    run_migration(&base, Some(&pid_path));

    // Both files should be cleaned up
    assert!(!pid_path.exists());
    assert!(!port_path.exists());

    let _ = fs::remove_dir_all(&base);
    let _ = fs::remove_dir_all(&legacy_dir);
}
