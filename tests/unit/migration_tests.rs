use copilot_adapter::daemon::status::{read_status_from, StatusFile};
use copilot_adapter::profile::migration::run_migration;
use std::fs;

/// Helper: create a unique temp directory for each test to avoid interference.
fn test_dir(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "copilot-adapter-migration-test-{}-{}",
        name,
        std::process::id()
    ));
    // Clean up any leftover from a prior run
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::create_dir_all(&dir);
    dir
}

// ============================================================
// No-op scenarios
// ============================================================

#[test]
fn migration_noop_when_profiles_dir_exists() {
    let base = test_dir("noop-profiles-exist");

    // Create profiles/ dir (simulates already migrated)
    fs::create_dir_all(base.join("profiles").join("default")).unwrap();

    // Create flat-dir files that should NOT be moved
    fs::write(base.join("status.json"), r#"{"pid":1,"port":1}"#).unwrap();
    fs::write(base.join("credentials.json"), b"creds").unwrap();

    run_migration(&base, None);

    // Flat files should still exist (not moved)
    assert!(base.join("status.json").exists());
    assert!(base.join("credentials.json").exists());

    let _ = fs::remove_dir_all(&base);
}

#[test]
fn migration_noop_when_no_flat_files() {
    let base = test_dir("noop-no-files");

    // Base dir exists but no flat files and no profiles/
    run_migration(&base, None);

    // profiles/ should NOT have been created
    assert!(!base.join("profiles").exists());

    let _ = fs::remove_dir_all(&base);
}

#[test]
fn migration_noop_when_no_flat_files_but_legacy_pid_missing() {
    let base = test_dir("noop-no-pid");
    let legacy_pid = base.join("legacy.pid");

    // No flat files, no legacy PID file
    run_migration(&base, Some(&legacy_pid));

    assert!(!base.join("profiles").exists());

    let _ = fs::remove_dir_all(&base);
}

// ============================================================
// Flat-dir file migration
// ============================================================

#[test]
fn migrate_status_json_to_default_profile() {
    let base = test_dir("migrate-status");

    let status = StatusFile {
        pid: std::process::id(),
        port: 6767,
        started_at: Some("2026-01-01T00:00:00+00:00".to_string()),
        version: Some("0.1.0".to_string()),
    };
    let json = serde_json::to_string_pretty(&status).unwrap();
    fs::write(base.join("status.json"), &json).unwrap();

    run_migration(&base, None);

    // Flat file should be gone
    assert!(
        !base.join("status.json").exists(),
        "flat status.json should be removed after migration"
    );

    // Profile file should exist with the same content
    let profile_status_path = base.join("profiles").join("default").join("status.json");
    assert!(
        profile_status_path.exists(),
        "status.json should be in profiles/default/"
    );

    let migrated = read_status_from(&profile_status_path).unwrap();
    assert_eq!(migrated.pid, std::process::id());
    assert_eq!(migrated.port, 6767);
    assert_eq!(
        migrated.started_at.as_deref(),
        Some("2026-01-01T00:00:00+00:00")
    );

    let _ = fs::remove_dir_all(&base);
}

#[test]
fn migrate_credentials_json_to_default_profile() {
    let base = test_dir("migrate-creds");

    fs::write(base.join("credentials.json"), b"encrypted-creds-data").unwrap();

    run_migration(&base, None);

    // Flat file should be gone
    assert!(
        !base.join("credentials.json").exists(),
        "flat credentials.json should be removed after migration"
    );

    // Profile file should exist with the same content
    let profile_creds_path = base
        .join("profiles")
        .join("default")
        .join("credentials.json");
    assert!(
        profile_creds_path.exists(),
        "credentials.json should be in profiles/default/"
    );

    let content = fs::read(&profile_creds_path).unwrap();
    assert_eq!(content, b"encrypted-creds-data");

    let _ = fs::remove_dir_all(&base);
}

#[test]
fn migrate_both_status_and_credentials() {
    let base = test_dir("migrate-both");

    fs::write(base.join("status.json"), r#"{"pid":1,"port":8080}"#).unwrap();
    fs::write(base.join("credentials.json"), b"creds").unwrap();

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

// ============================================================
// Idempotency
// ============================================================

#[test]
fn migration_is_idempotent() {
    let base = test_dir("idempotent");

    fs::write(base.join("status.json"), r#"{"pid":1,"port":6767}"#).unwrap();
    fs::write(base.join("credentials.json"), b"creds").unwrap();

    // First migration
    run_migration(&base, None);

    let default_dir = base.join("profiles").join("default");
    assert!(default_dir.join("status.json").exists());
    assert!(default_dir.join("credentials.json").exists());

    let status_content = fs::read_to_string(default_dir.join("status.json")).unwrap();
    let creds_content = fs::read(default_dir.join("credentials.json")).unwrap();

    // Second migration — should be a no-op because profiles/ now exists
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

// ============================================================
// Legacy PID file migration
// ============================================================

#[test]
fn migrate_legacy_pid_dead_process_cleans_up() {
    let base = test_dir("legacy-dead");
    let legacy_dir = std::env::temp_dir().join(format!(
        "copilot-adapter-migration-legacy-dead-{}",
        std::process::id()
    ));
    let _ = fs::create_dir_all(&legacy_dir);
    let pid_path = legacy_dir.join("copilot-adapter.pid");
    let port_path = legacy_dir.join("copilot-adapter.port");

    // Write a PID for a dead process
    fs::write(&pid_path, "99999999").unwrap();
    fs::write(&port_path, "7070").unwrap();

    // Also create a flat-dir file so migration triggers
    // (without flat files AND without a running legacy PID, there's nothing to do
    //  but here the PID file exists which triggers migration)
    run_migration(&base, Some(&pid_path));

    // Legacy files should be cleaned up
    assert!(
        !pid_path.exists(),
        "legacy PID file should be removed for dead process"
    );
    assert!(
        !port_path.exists(),
        "legacy port file should be removed for dead process"
    );

    // No status.json should be created for a dead process
    let status_path = base.join("profiles").join("default").join("status.json");
    assert!(
        !status_path.exists(),
        "should not create status.json for dead process"
    );

    let _ = fs::remove_dir_all(&base);
    let _ = fs::remove_dir_all(&legacy_dir);
}

#[test]
fn migrate_legacy_pid_running_process_synthesizes_status() {
    let base = test_dir("legacy-running");
    let legacy_dir = std::env::temp_dir().join(format!(
        "copilot-adapter-migration-legacy-running-{}",
        std::process::id()
    ));
    let _ = fs::create_dir_all(&legacy_dir);
    let pid_path = legacy_dir.join("copilot-adapter.pid");
    let port_path = legacy_dir.join("copilot-adapter.port");

    // Write current process PID (definitely running)
    let current_pid = std::process::id();
    fs::write(&pid_path, current_pid.to_string()).unwrap();
    fs::write(&port_path, "9090").unwrap();

    run_migration(&base, Some(&pid_path));

    // Legacy files should be cleaned up after synthesis
    assert!(!pid_path.exists(), "legacy PID file should be removed");
    assert!(!port_path.exists(), "legacy port file should be removed");

    // A synthesized status.json should exist
    let status_path = base.join("profiles").join("default").join("status.json");
    assert!(
        status_path.exists(),
        "should create status.json for running process"
    );

    let status = read_status_from(&status_path).unwrap();
    assert_eq!(status.pid, current_pid);
    assert_eq!(status.port, 9090);
    // Synthesized entries have no started_at or version
    assert!(status.started_at.is_none());
    assert!(status.version.is_none());

    let _ = fs::remove_dir_all(&base);
    let _ = fs::remove_dir_all(&legacy_dir);
}

#[test]
fn migrate_legacy_pid_skipped_when_status_already_migrated() {
    let base = test_dir("legacy-skip");
    let legacy_dir = std::env::temp_dir().join(format!(
        "copilot-adapter-migration-legacy-skip-{}",
        std::process::id()
    ));
    let _ = fs::create_dir_all(&legacy_dir);
    let pid_path = legacy_dir.join("copilot-adapter.pid");
    let port_path = legacy_dir.join("copilot-adapter.port");

    // Write flat-dir status (will be migrated first)
    let original_status = r#"{"pid":42,"port":6767,"started_at":"2026-01-01T00:00:00+00:00","version":"1.0.0"}"#;
    fs::write(base.join("status.json"), original_status).unwrap();

    // Write legacy PID (current process, definitely running)
    fs::write(&pid_path, std::process::id().to_string()).unwrap();
    fs::write(&port_path, "9999").unwrap();

    run_migration(&base, Some(&pid_path));

    // Legacy files should be cleaned up
    assert!(!pid_path.exists());
    assert!(!port_path.exists());

    // The status.json should contain the flat-dir data (not synthesized from PID)
    let status_path = base.join("profiles").join("default").join("status.json");
    let status = read_status_from(&status_path).unwrap();
    assert_eq!(status.pid, 42, "should keep flat-dir status, not legacy PID");
    assert_eq!(
        status.port, 6767,
        "should keep flat-dir port, not legacy port"
    );

    let _ = fs::remove_dir_all(&base);
    let _ = fs::remove_dir_all(&legacy_dir);
}

#[test]
fn migrate_legacy_pid_invalid_content_cleans_up() {
    let base = test_dir("legacy-invalid");
    let legacy_dir = std::env::temp_dir().join(format!(
        "copilot-adapter-migration-legacy-invalid-{}",
        std::process::id()
    ));
    let _ = fs::create_dir_all(&legacy_dir);
    let pid_path = legacy_dir.join("copilot-adapter.pid");
    let port_path = pid_path.with_extension("port");

    // Write invalid PID content and a companion port file
    fs::write(&pid_path, "not-a-pid").unwrap();
    fs::write(&port_path, "7070").unwrap();

    run_migration(&base, Some(&pid_path));

    // Invalid PID file should be cleaned up
    assert!(
        !pid_path.exists(),
        "invalid PID file should be cleaned up"
    );
    // Port file should also be cleaned up
    assert!(
        !port_path.exists(),
        "port file should also be cleaned up for invalid PID content"
    );

    let _ = fs::remove_dir_all(&base);
    let _ = fs::remove_dir_all(&legacy_dir);
}

#[test]
fn migrate_legacy_pid_without_port_file_uses_zero() {
    let base = test_dir("legacy-no-port");
    let legacy_dir = std::env::temp_dir().join(format!(
        "copilot-adapter-migration-legacy-noport-{}",
        std::process::id()
    ));
    let _ = fs::create_dir_all(&legacy_dir);
    let pid_path = legacy_dir.join("copilot-adapter.pid");

    // Write current process PID but no port file
    fs::write(&pid_path, std::process::id().to_string()).unwrap();

    run_migration(&base, Some(&pid_path));

    // Status should exist with port 0
    let status_path = base.join("profiles").join("default").join("status.json");
    let status = read_status_from(&status_path).unwrap();
    assert_eq!(status.pid, std::process::id());
    assert_eq!(status.port, 0, "port should default to 0 when port file is missing");

    let _ = fs::remove_dir_all(&base);
    let _ = fs::remove_dir_all(&legacy_dir);
}

// ============================================================
// Combined migration scenarios
// ============================================================

#[test]
fn migrate_flat_files_and_legacy_pid_together() {
    let base = test_dir("combined");
    let legacy_dir = std::env::temp_dir().join(format!(
        "copilot-adapter-migration-combined-{}",
        std::process::id()
    ));
    let _ = fs::create_dir_all(&legacy_dir);
    let pid_path = legacy_dir.join("copilot-adapter.pid");
    let port_path = legacy_dir.join("copilot-adapter.port");

    // Flat-dir files
    let original_status = r#"{"pid":12345,"port":6767}"#;
    fs::write(base.join("status.json"), original_status).unwrap();
    fs::write(base.join("credentials.json"), b"token-data").unwrap();

    // Legacy PID file (running process)
    fs::write(&pid_path, std::process::id().to_string()).unwrap();
    fs::write(&port_path, "8080").unwrap();

    run_migration(&base, Some(&pid_path));

    let default_dir = base.join("profiles").join("default");

    // Flat files should be moved
    assert!(!base.join("status.json").exists());
    assert!(!base.join("credentials.json").exists());
    assert!(default_dir.join("status.json").exists());
    assert!(default_dir.join("credentials.json").exists());

    // Legacy PID files should be cleaned up
    assert!(!pid_path.exists());
    assert!(!port_path.exists());

    // Status should contain flat-dir data (not synthesized from PID)
    let status = read_status_from(&default_dir.join("status.json")).unwrap();
    assert_eq!(status.pid, 12345);
    assert_eq!(status.port, 6767);

    // Credentials should be intact
    let creds = fs::read(default_dir.join("credentials.json")).unwrap();
    assert_eq!(creds, b"token-data");

    let _ = fs::remove_dir_all(&base);
    let _ = fs::remove_dir_all(&legacy_dir);
}

#[test]
fn migrate_only_credentials_no_status() {
    let base = test_dir("creds-only");

    // Only credentials.json, no status.json
    fs::write(base.join("credentials.json"), b"my-creds").unwrap();

    run_migration(&base, None);

    assert!(!base.join("credentials.json").exists());
    assert!(
        base.join("profiles")
            .join("default")
            .join("credentials.json")
            .exists()
    );
    // No status.json should exist in profile dir
    assert!(!base
        .join("profiles")
        .join("default")
        .join("status.json")
        .exists());

    let _ = fs::remove_dir_all(&base);
}
