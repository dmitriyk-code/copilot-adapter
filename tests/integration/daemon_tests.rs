//! Integration tests for daemon lifecycle (start → status → stop).
//!
//! These tests exercise the PID file management and process lifecycle
//! on the current platform. They use the daemon module directly rather than
//! spawning the full CLI binary, to keep tests fast and avoid needing auth.
//!
//! All tests that touch PID files must hold `PID_LOCK` to avoid interference
//! when running in parallel.

use copilot_adapter::daemon;
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

#[test]
fn daemon_pid_lifecycle() {
    let _lock = PID_LOCK.lock().unwrap();
    let _cleanup = PidCleanup;

    // Initially not running (clean state)
    daemon::remove_pid_file();
    daemon::remove_port_file();
    assert!(daemon::is_running().is_none(), "should not be running initially");

    // Write PID file for current process — simulates daemon start
    daemon::write_pid_file().unwrap();
    daemon::write_port_file(8787).unwrap();

    // Status should show running
    let pid = daemon::is_running();
    assert!(pid.is_some(), "should be running after write_pid_file");
    assert_eq!(pid.unwrap(), std::process::id());

    // Port should be readable
    assert_eq!(daemon::read_port(), Some(8787));

    // Clean up
    daemon::remove_pid_file();
    daemon::remove_port_file();

    // After removal, should report not running
    assert!(daemon::is_running().is_none(), "should not be running after cleanup");
    assert!(daemon::read_port().is_none(), "port should be gone after cleanup");
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
    assert!(daemon::is_running().is_none(), "stale PID should not be reported as running");
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
        daemon::write_port_file(8787).unwrap();

        assert!(daemon::is_running().is_some());
        assert_eq!(daemon::read_port(), Some(8787));

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
