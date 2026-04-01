use copilot_adapter::daemon::status::{
    get_base_dir, get_default_status_path, read_status_from, remove_status_from, write_status_to,
    StatusFile,
};
use copilot_adapter::daemon;
use std::fs;

/// Helper: create a unique temp directory for each test to avoid interference.
fn test_dir(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "copilot-adapter-status-test-{}-{}",
        name,
        std::process::id()
    ));
    let _ = fs::create_dir_all(&dir);
    dir
}

#[test]
fn status_file_serialization_round_trip() {
    let status = StatusFile {
        pid: 12345,
        port: 6767,
        started_at: Some("2026-01-01T00:00:00+00:00".to_string()),
        version: Some("0.1.0".to_string()),
    };

    let json = serde_json::to_string_pretty(&status).unwrap();
    let deserialized: StatusFile = serde_json::from_str(&json).unwrap();

    assert_eq!(deserialized.pid, 12345);
    assert_eq!(deserialized.port, 6767);
    assert_eq!(
        deserialized.started_at.as_deref(),
        Some("2026-01-01T00:00:00+00:00")
    );
    assert_eq!(deserialized.version.as_deref(), Some("0.1.0"));
}

#[test]
fn status_file_serialization_omits_none_fields() {
    let status = StatusFile {
        pid: 42,
        port: 0,
        started_at: None,
        version: None,
    };

    let json = serde_json::to_string(&status).unwrap();
    // None fields should be omitted from JSON output
    assert!(!json.contains("started_at"));
    assert!(!json.contains("version"));
    // Present fields should still be there
    assert!(json.contains("\"pid\""));
    assert!(json.contains("\"port\""));
}

#[test]
fn status_file_deserializes_from_known_json() {
    let json = r#"{
        "pid": 42,
        "port": 8080,
        "started_at": "2026-03-15T12:30:00+00:00",
        "version": "1.2.3"
    }"#;

    let status: StatusFile = serde_json::from_str(json).unwrap();
    assert_eq!(status.pid, 42);
    assert_eq!(status.port, 8080);
    assert_eq!(
        status.started_at.as_deref(),
        Some("2026-03-15T12:30:00+00:00")
    );
    assert_eq!(status.version.as_deref(), Some("1.2.3"));
}

#[test]
fn status_file_deserializes_without_optional_fields() {
    let json = r#"{"pid": 99, "port": 3000}"#;

    let status: StatusFile = serde_json::from_str(json).unwrap();
    assert_eq!(status.pid, 99);
    assert_eq!(status.port, 3000);
    assert!(status.started_at.is_none());
    assert!(status.version.is_none());
}

#[test]
fn get_base_dir_returns_valid_directory() {
    let dir = get_base_dir();
    // Should exist and be a directory (get_base_dir creates it)
    assert!(dir.exists());
    assert!(dir.is_dir());
}

#[test]
fn get_default_status_path_ends_with_status_json() {
    let path = get_default_status_path();
    assert_eq!(path.file_name().unwrap(), "status.json");
}

#[test]
fn write_read_remove_lifecycle() {
    let dir = test_dir("lifecycle");
    let path = dir.join("status.json");

    // Initially no status file
    assert!(read_status_from(&path).is_none());

    // Write
    write_status_to(&path, 6767).unwrap();
    assert!(path.exists());

    // Read
    let status = read_status_from(&path).unwrap();
    assert_eq!(status.pid, std::process::id());
    assert_eq!(status.port, 6767);
    assert_eq!(
        status.version.as_deref(),
        Some(env!("CARGO_PKG_VERSION"))
    );
    // started_at should be a non-empty ISO 8601 string
    assert!(status.started_at.is_some());
    assert!(!status.started_at.as_ref().unwrap().is_empty());

    // Remove
    remove_status_from(&path);
    assert!(!path.exists());
    assert!(read_status_from(&path).is_none());

    // Cleanup
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn write_creates_parent_directories() {
    let dir = test_dir("mkdir");
    let nested = dir.join("nested").join("deep").join("status.json");

    write_status_to(&nested, 9090).unwrap();
    assert!(nested.exists());

    let status = read_status_from(&nested).unwrap();
    assert_eq!(status.port, 9090);

    // Cleanup
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn read_status_from_returns_none_for_missing_file() {
    let path = std::env::temp_dir().join(format!(
        "nonexistent-status-{}.json",
        std::process::id()
    ));
    assert!(read_status_from(&path).is_none());
}

#[test]
fn read_status_from_returns_none_for_invalid_json() {
    let dir = test_dir("invalid-json");
    let path = dir.join("status.json");

    fs::write(&path, "this is not json").unwrap();
    assert!(read_status_from(&path).is_none());

    // Cleanup
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn remove_status_from_is_idempotent() {
    let dir = test_dir("idempotent");
    let path = dir.join("status.json");

    // Remove on non-existent file should not panic
    remove_status_from(&path);
    remove_status_from(&path);

    // Write then remove twice
    write_status_to(&path, 6767).unwrap();
    remove_status_from(&path);
    remove_status_from(&path);

    // Cleanup
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn write_status_overwrites_existing() {
    let dir = test_dir("overwrite");
    let path = dir.join("status.json");

    write_status_to(&path, 1111).unwrap();
    let status1 = read_status_from(&path).unwrap();
    assert_eq!(status1.port, 1111);

    write_status_to(&path, 2222).unwrap();
    let status2 = read_status_from(&path).unwrap();
    assert_eq!(status2.port, 2222);

    // Cleanup
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn status_file_contains_current_pid() {
    let dir = test_dir("current-pid");
    let path = dir.join("status.json");

    write_status_to(&path, 6767).unwrap();
    let status = read_status_from(&path).unwrap();
    assert_eq!(status.pid, std::process::id());

    // Cleanup
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn status_file_version_matches_cargo_pkg() {
    let dir = test_dir("version");
    let path = dir.join("status.json");

    write_status_to(&path, 6767).unwrap();
    let status = read_status_from(&path).unwrap();
    assert_eq!(
        status.version.as_deref(),
        Some(env!("CARGO_PKG_VERSION"))
    );

    // Cleanup
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn status_file_started_at_is_iso8601() {
    let dir = test_dir("timestamp");
    let path = dir.join("status.json");

    write_status_to(&path, 6767).unwrap();
    let status = read_status_from(&path).unwrap();

    // Verify it parses as a valid RFC3339/ISO8601 timestamp
    let started_at = status.started_at.as_ref().expect("started_at should be Some");
    let parsed = chrono::DateTime::parse_from_rfc3339(started_at);
    assert!(
        parsed.is_ok(),
        "started_at should be valid RFC3339: {}",
        started_at
    );

    // Cleanup
    let _ = fs::remove_dir_all(&dir);
}

// --- Tests for is_running_from_status() and remove_all_status_files() ---
//
// These tests operate on the real default status path and legacy PID/port paths.
// They are combined into a single test to avoid parallel-execution interference.

#[test]
fn is_running_from_status_and_cleanup_integration() {
    // Ensure clean state — remove any leftovers from prior runs.
    daemon::remove_all_status_files();

    // ------ Part 1: is_running_from_status returns Some for current process ------
    let status = StatusFile {
        pid: std::process::id(),
        port: 7777,
        started_at: Some("2026-01-01T00:00:00+00:00".to_string()),
        version: Some("0.1.0".to_string()),
    };
    let path = daemon::status::get_default_status_path();
    if let Some(parent) = path.parent() {
        let _ = fs::create_dir_all(parent);
    }
    fs::write(&path, serde_json::to_string_pretty(&status).unwrap()).unwrap();

    let result = daemon::is_running_from_status();
    assert!(result.is_some(), "should detect current process as running");
    let found = result.unwrap();
    assert_eq!(found.pid, std::process::id());
    assert_eq!(found.port, 7777);

    // Clean up for next part
    let _ = fs::remove_file(&path);

    // ------ Part 2: is_running_from_status cleans up stale file ------
    let dead_pid: u32 = u32::MAX - 1; // extremely unlikely to be a real PID
    let stale_status = StatusFile {
        pid: dead_pid,
        port: 9999,
        started_at: Some("2026-01-01T00:00:00+00:00".to_string()),
        version: Some("0.1.0".to_string()),
    };
    fs::write(
        &path,
        serde_json::to_string_pretty(&stale_status).unwrap(),
    )
    .unwrap();
    assert!(path.exists(), "status file should exist before check");

    // Ensure no legacy PID file interferes
    let legacy_pid_path = daemon::get_pid_path();
    let _ = fs::remove_file(&legacy_pid_path);

    let result = daemon::is_running_from_status();
    assert!(result.is_none(), "should return None for dead process");
    assert!(!path.exists(), "stale status file should be cleaned up");

    // ------ Part 3: remove_all_status_files removes new and legacy files ------
    // Create the new status file
    let status_path = daemon::status::get_default_status_path();
    fs::write(&status_path, r#"{"pid":1,"port":1}"#).unwrap();

    // Create legacy PID and port files
    let pid_path = daemon::get_pid_path();
    let port_path = pid_path.with_extension("port");
    fs::write(&pid_path, "12345").unwrap();
    fs::write(&port_path, "6767").unwrap();

    assert!(status_path.exists());
    assert!(pid_path.exists());
    assert!(port_path.exists());

    daemon::remove_all_status_files();

    assert!(!status_path.exists(), "status.json should be removed");
    assert!(!pid_path.exists(), "legacy PID file should be removed");
    assert!(!port_path.exists(), "legacy port file should be removed");
}
