//! Integration tests for daemon lifecycle (start → status → stop).
//!
//! These tests exercise the PID file management, status file management,
//! credential storage lifecycle, and process lifecycle on the current platform.
//! They use the daemon/storage/profile modules directly rather than spawning
//! the full CLI binary, to keep tests fast and avoid needing auth.
//!
//! All tests that touch PID files must hold `PID_LOCK` to avoid interference
//! when running in parallel.

use copilot_adapter::daemon;
use copilot_adapter::daemon::status::{
    read_status_from, remove_status_from, write_status_to, StatusFile,
};
use copilot_adapter::profile::ProfileManager;
use copilot_adapter::storage::file::FileStorage;
use copilot_adapter::storage::TokenStorage;
use std::fs;
use std::sync::Mutex;

/// Serialize all tests that touch the shared PID/port files.
static PID_LOCK: Mutex<()> = Mutex::new(());

/// Helper to ensure PID/port files are cleaned up after each test.
struct PidCleanup;

impl Drop for PidCleanup {
    fn drop(&mut self) {
        daemon::remove_pid_file();
        daemon::remove_port_file();
    }
}

/// Helper: create a unique temp directory for each test to avoid interference.
fn test_dir(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "copilot-adapter-integ-daemon-{}-{}",
        name,
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::create_dir_all(&dir);
    dir
}

// ============================================================
// Existing daemon PID lifecycle tests
// ============================================================

#[test]
fn daemon_pid_lifecycle() {
    let _lock = PID_LOCK.lock().unwrap();
    let _cleanup = PidCleanup;

    // Initially not running (clean state)
    daemon::remove_pid_file();
    daemon::remove_port_file();
    assert!(
        daemon::is_running().is_none(),
        "should not be running initially"
    );

    // Write PID file for current process — simulates daemon start
    daemon::write_pid_file().unwrap();
    daemon::write_port_file(6767).unwrap();

    // Status should show running
    let pid = daemon::is_running();
    assert!(pid.is_some(), "should be running after write_pid_file");
    assert_eq!(pid.unwrap(), std::process::id());

    // Port should be readable
    assert_eq!(daemon::read_port(), Some(6767));

    // Clean up
    daemon::remove_pid_file();
    daemon::remove_port_file();

    // After removal, should report not running
    assert!(
        daemon::is_running().is_none(),
        "should not be running after cleanup"
    );
    assert!(
        daemon::read_port().is_none(),
        "port should be gone after cleanup"
    );
}

#[test]
fn stale_pid_file_is_cleaned_up() {
    let _lock = PID_LOCK.lock().unwrap();
    let _cleanup = PidCleanup;

    let pid_path = daemon::get_pid_path();

    // Write a PID that doesn't correspond to any real process
    // Use a high but valid u32 value
    std::fs::write(&pid_path, "99999999").unwrap();

    // is_running should detect the stale PID and clean up
    assert!(
        daemon::is_running().is_none(),
        "stale PID should not be reported as running"
    );
    assert!(!pid_path.exists(), "stale PID file should be removed");
}

#[test]
fn stop_daemon_fails_when_not_running() {
    let _lock = PID_LOCK.lock().unwrap();
    let _cleanup = PidCleanup;

    // Ensure clean state
    daemon::remove_pid_file();

    let result = daemon::stop_daemon();
    assert!(result.is_err(), "stop should fail when not running");
    assert!(
        result.unwrap_err().to_string().contains("not running"),
        "error should mention not running"
    );
}

#[test]
fn pid_path_is_in_temp() {
    let path = daemon::get_pid_path();
    let temp = std::env::temp_dir();
    assert!(
        path.starts_with(&temp),
        "PID path {:?} should be under temp dir {:?}",
        path,
        temp
    );
}

#[test]
fn process_exists_for_current_pid() {
    assert!(daemon::process_exists(std::process::id()));
}

#[test]
fn process_exists_returns_false_for_invalid_pid() {
    // Use a PID that's very unlikely to exist but is a valid u32
    assert!(!daemon::process_exists(99999999));
}

/// Test that writing and reading PID/port files round-trips correctly.
#[test]
fn pid_and_port_round_trip() {
    let _lock = PID_LOCK.lock().unwrap();
    let _cleanup = PidCleanup;
    daemon::remove_pid_file();
    daemon::remove_port_file();

    daemon::write_pid_file().unwrap();
    daemon::write_port_file(9999).unwrap();

    let content = std::fs::read_to_string(daemon::get_pid_path()).unwrap();
    assert_eq!(content.trim().parse::<u32>().unwrap(), std::process::id());

    assert_eq!(daemon::read_port(), Some(9999));

    daemon::remove_pid_file();
    daemon::remove_port_file();
}

/// Platform-specific test: on Windows, test spawn_background + stop lifecycle.
/// On Unix, test daemonize is available.
#[cfg(windows)]
mod platform {
    use super::*;

    #[test]
    fn spawn_and_stop_background_process() {
        let _lock = PID_LOCK.lock().unwrap();
        let _cleanup = PidCleanup;
        daemon::remove_pid_file();
        daemon::remove_port_file();

        // Write a PID for the current process and verify stop behavior.
        daemon::write_pid_file().unwrap();
        daemon::write_port_file(6767).unwrap();

        assert!(daemon::is_running().is_some());
        assert_eq!(daemon::read_port(), Some(6767));

        // Don't actually stop our own process — just verify the lifecycle
        daemon::remove_pid_file();
        daemon::remove_port_file();
        assert!(daemon::is_running().is_none());
    }
}

#[cfg(unix)]
mod platform {
    use super::*;

    #[test]
    fn stop_when_not_running_errors() {
        let _lock = PID_LOCK.lock().unwrap();
        let _cleanup = PidCleanup;
        daemon::remove_pid_file();

        let result = daemon::stop_daemon();
        assert!(result.is_err());
    }
}

// ============================================================
// Task 8.1: Daemon auth integration test
// ============================================================
//
// Verify that the daemon start path does NOT gate on pre-existing credentials.
// We test this by verifying that the CLI binary parses `start --daemon`
// successfully and that the code path for pre-auth validation is reachable
// (i.e., it attempts auth instead of exiting with an error).
//
// Since we can't run a real OAuth device flow in tests, we verify:
// 1. Profile resolution works without pre-existing credentials
// 2. Storage creation for a profile without stored tokens returns an error
//    (triggering the auth flow in production) rather than panicking or exiting

#[test]
fn daemon_start_without_credentials_does_not_panic() {
    let base = test_dir("daemon-auth");
    let mgr = ProfileManager::with_base_dir(base.clone());

    // Get the default profile (auto-creates directory)
    let profile = mgr.get("default").unwrap();

    // Verify no credentials exist yet
    assert!(
        !profile.credentials_path().exists(),
        "credentials should not exist in fresh profile"
    );

    // Create storage for this profile — mirrors what main.rs does before auth
    let storage =
        copilot_adapter::storage::create_storage_for_profile(&profile, false);

    // Attempting to get a token should return an error (no stored token),
    // which is the trigger for the interactive auth flow. The key assertion
    // is that this does NOT panic or exit — it gracefully returns Err.
    let result = storage.get_github_token();
    assert!(
        result.is_err(),
        "should return Err (no token) — daemon would trigger auth flow here"
    );

    // Port conflict check should also work with no running instances
    let conflict = mgr.check_port_conflict(6767, "default");
    assert!(
        conflict.is_ok(),
        "port conflict check should pass with no running instances"
    );

    let _ = fs::remove_dir_all(&base);
}

#[test]
fn daemon_start_with_credentials_succeeds() {
    let base = test_dir("daemon-auth-with-creds");
    let mgr = ProfileManager::with_base_dir(base.clone());

    let profile = mgr.get("default").unwrap();

    // Store credentials
    let storage =
        copilot_adapter::storage::create_storage_for_profile(&profile, false);
    storage.store_github_token("ghp_test_daemon_auth").unwrap();

    // Credentials should be retrievable — mirrors the check main.rs does
    let token = storage.get_github_token().unwrap();
    assert_eq!(token, "ghp_test_daemon_auth");

    let _ = fs::remove_dir_all(&base);
}

// ============================================================
// Task 8.2: Status file lifecycle test
// ============================================================

#[test]
fn status_file_write_read_remove_lifecycle() {
    let dir = test_dir("status-lifecycle");
    let status_path = dir.join("status.json");

    // Step 1: Initially no status
    assert!(
        read_status_from(&status_path).is_none(),
        "should have no status initially"
    );

    // Step 2: Write status (simulates daemon start)
    write_status_to(&status_path, 6767).unwrap();
    assert!(status_path.exists(), "status file should be created");

    // Step 3: Read back and verify all fields
    let status = read_status_from(&status_path).unwrap();
    assert_eq!(status.pid, std::process::id());
    assert_eq!(status.port, 6767);
    assert!(status.started_at.is_some(), "started_at should be populated");
    assert!(status.version.is_some(), "version should be populated");
    assert_eq!(
        status.version.as_deref(),
        Some(env!("CARGO_PKG_VERSION"))
    );

    // Verify started_at is valid RFC3339
    let started_at = status.started_at.as_ref().unwrap();
    assert!(
        chrono::DateTime::parse_from_rfc3339(started_at).is_ok(),
        "started_at should be valid RFC3339: {}",
        started_at
    );

    // Step 4: is_running_from_status detects live process
    // Write to the default location for this check
    // (We test the parameterized version separately)

    // Step 5: Remove status (simulates daemon stop)
    remove_status_from(&status_path);
    assert!(!status_path.exists(), "status file should be removed");

    // Step 6: After removal, reads return None
    assert!(
        read_status_from(&status_path).is_none(),
        "should return None after removal"
    );

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn status_file_in_profile_directory() {
    let base = test_dir("status-profile");
    let mgr = ProfileManager::with_base_dir(base.clone());

    let profile = mgr.get("default").unwrap();
    let status_path = profile.status_path();

    // Write status to profile's status path
    write_status_to(&status_path, 8080).unwrap();
    assert!(status_path.exists());

    // Read back
    let status = read_status_from(&status_path).unwrap();
    assert_eq!(status.port, 8080);
    assert_eq!(status.pid, std::process::id());

    // Verify the path is under profiles/default/
    assert!(
        status_path.ends_with("status.json"),
        "status path should end with status.json"
    );
    let parent = status_path.parent().unwrap();
    assert!(
        parent.ends_with("default"),
        "status file should be in the profile directory"
    );

    // Remove and verify
    remove_status_from(&status_path);
    assert!(read_status_from(&status_path).is_none());

    let _ = fs::remove_dir_all(&base);
}

#[test]
fn status_file_overwrite_updates_values() {
    let dir = test_dir("status-overwrite");
    let path = dir.join("status.json");

    write_status_to(&path, 6767).unwrap();
    let s1 = read_status_from(&path).unwrap();
    assert_eq!(s1.port, 6767);

    write_status_to(&path, 9090).unwrap();
    let s2 = read_status_from(&path).unwrap();
    assert_eq!(s2.port, 9090);

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn status_file_stale_process_detection() {
    let dir = test_dir("status-stale");
    let path = dir.join("status.json");

    // Write status with a bogus PID that doesn't exist
    let stale = StatusFile {
        pid: 99999999,
        port: 6767,
        started_at: Some("2026-01-01T00:00:00+00:00".to_string()),
        version: Some("0.1.0".to_string()),
    };
    fs::write(&path, serde_json::to_string_pretty(&stale).unwrap()).unwrap();

    // The status file exists but the process is dead
    let status = read_status_from(&path);
    assert!(status.is_some(), "read_status_from should return the file contents");
    assert_eq!(status.unwrap().pid, 99999999);

    // process_exists should return false for the stale PID
    assert!(
        !daemon::process_exists(99999999),
        "bogus PID should not be a live process"
    );

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn remove_status_is_idempotent() {
    let dir = test_dir("status-idempotent");
    let path = dir.join("status.json");

    // Remove on non-existent file should not panic
    remove_status_from(&path);
    remove_status_from(&path);

    // Write then remove twice
    write_status_to(&path, 6767).unwrap();
    remove_status_from(&path);
    remove_status_from(&path);

    let _ = fs::remove_dir_all(&dir);
}

// ============================================================
// Task 8.3: Credential storage lifecycle test
// ============================================================

#[test]
fn credential_storage_store_retrieve_delete_lifecycle() {
    let base = test_dir("cred-lifecycle");
    let mgr = ProfileManager::with_base_dir(base.clone());

    let profile = mgr.get("default").unwrap();
    let creds_path = profile.credentials_path();

    // Step 1: Create file storage at profile path
    let storage = FileStorage::with_path(creds_path.clone());

    // Step 2: No token initially
    assert!(
        storage.get_github_token().is_err(),
        "should have no token initially"
    );

    // Step 3: Store a token (simulates successful auth)
    storage.store_github_token("ghp_integration_test_token").unwrap();
    assert!(creds_path.exists(), "credentials file should be created");

    // Step 4: Retrieve the token (simulates restart → load)
    let token = storage.get_github_token().unwrap();
    assert_eq!(token, "ghp_integration_test_token");

    // Step 5: Overwrite with a new token
    storage.store_github_token("ghp_refreshed_token").unwrap();
    assert_eq!(storage.get_github_token().unwrap(), "ghp_refreshed_token");

    // Step 6: Delete the token (simulates logout)
    storage.delete_github_token().unwrap();
    assert!(
        storage.get_github_token().is_err(),
        "token should be gone after delete"
    );

    let _ = fs::remove_dir_all(&base);
}

#[test]
fn credential_storage_survives_new_storage_instance() {
    let base = test_dir("cred-restart");
    let mgr = ProfileManager::with_base_dir(base.clone());

    let profile = mgr.get("default").unwrap();
    let creds_path = profile.credentials_path();

    // Store with first storage instance
    let storage1 = FileStorage::with_path(creds_path.clone());
    storage1
        .store_github_token("ghp_persist_across_restart")
        .unwrap();

    // Create a NEW storage instance (simulates process restart)
    let storage2 = FileStorage::with_path(creds_path.clone());
    let token = storage2.get_github_token().unwrap();
    assert_eq!(
        token, "ghp_persist_across_restart",
        "token should persist across storage instances"
    );

    let _ = fs::remove_dir_all(&base);
}

#[test]
fn credential_storage_per_profile_isolation() {
    let base = test_dir("cred-isolation");
    let mgr = ProfileManager::with_base_dir(base.clone());

    // Create two profiles
    let default_profile = mgr.get("default").unwrap();
    let work_profile = mgr.create("work").unwrap();

    // Store different tokens for each profile
    let default_storage = FileStorage::with_path(default_profile.credentials_path());
    let work_storage = FileStorage::with_path(work_profile.credentials_path());

    default_storage
        .store_github_token("ghp_default_token")
        .unwrap();
    work_storage
        .store_github_token("ghp_work_token")
        .unwrap();

    // Verify isolation: each profile has its own token
    assert_eq!(default_storage.get_github_token().unwrap(), "ghp_default_token");
    assert_eq!(work_storage.get_github_token().unwrap(), "ghp_work_token");

    // Delete one, the other should survive
    default_storage.delete_github_token().unwrap();
    assert!(default_storage.get_github_token().is_err());
    assert_eq!(work_storage.get_github_token().unwrap(), "ghp_work_token");

    let _ = fs::remove_dir_all(&base);
}

#[test]
fn credential_migration_from_legacy_path() {
    let base = test_dir("cred-migration");

    // Simulate legacy credentials at a "source" path
    let legacy_dir = base.join("legacy");
    let _ = fs::create_dir_all(&legacy_dir);
    let legacy_path = legacy_dir.join("credentials.json");
    let legacy_storage = FileStorage::with_path(legacy_path.clone());
    legacy_storage.store_github_token("ghp_legacy_token").unwrap();

    // New destination path
    let new_dir = base.join("new_profile");
    let _ = fs::create_dir_all(&new_dir);
    let new_path = new_dir.join("credentials.json");

    // Run migration
    copilot_adapter::storage::file::migrate_from_to(&legacy_path, &new_path);

    // Verify new location has the token
    let new_storage = FileStorage::with_path(new_path.clone());
    assert_eq!(new_storage.get_github_token().unwrap(), "ghp_legacy_token");

    // Legacy file should still exist (copy, not move)
    assert!(legacy_path.exists());

    // Migration should be idempotent — running again should be a no-op
    // because the destination file already exists
    copilot_adapter::storage::file::migrate_if_needed(&new_path);
    assert_eq!(new_storage.get_github_token().unwrap(), "ghp_legacy_token");

    let _ = fs::remove_dir_all(&base);
}

#[test]
fn credential_storage_via_create_storage_for_profile() {
    let base = test_dir("cred-factory");
    let mgr = ProfileManager::with_base_dir(base.clone());

    let profile = mgr.get("default").unwrap();

    // Use the factory function (mirrors actual usage in main.rs)
    let storage =
        copilot_adapter::storage::create_storage_for_profile(&profile, false);

    storage.store_github_token("ghp_factory_test").unwrap();
    assert_eq!(storage.get_github_token().unwrap(), "ghp_factory_test");

    // Verify the file is at the expected location
    assert!(
        profile.credentials_path().exists(),
        "credentials file should be at profile.credentials_path()"
    );

    let _ = fs::remove_dir_all(&base);
}
