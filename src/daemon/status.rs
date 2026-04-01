//! Status file management for the copilot-adapter daemon.
//!
//! Manages `~/.copilot-adapter/status.json` containing PID, port, version,
//! and start time. Provides both parameterized APIs (for future profile support)
//! and convenience wrappers that use a default path.

use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// Runtime status of a running copilot-adapter instance.
///
/// Fields `started_at` and `version` are `Option` because they are unavailable
/// when synthesised from legacy PID files (backward-compat path). Port is 0
/// only when the legacy port file is missing.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StatusFile {
    pub pid: u32,
    pub port: u16,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub started_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
}

/// Returns the base directory: `~/.copilot-adapter/`.
///
/// Falls back to the OS temp directory if the home directory is unavailable
/// or not writable.
pub fn get_base_dir() -> PathBuf {
    if let Some(home) = dirs::home_dir() {
        let dir = home.join(".copilot-adapter");
        if std::fs::create_dir_all(&dir).is_ok() {
            return dir;
        }
    }
    std::env::temp_dir()
}

/// Default status file path (non-profile mode): `~/.copilot-adapter/status.json`.
pub fn get_default_status_path() -> PathBuf {
    get_base_dir().join("status.json")
}

// --- Parameterized API (used directly in Epic 4+ for profile support) ---

/// Write a status file at the given path with the current process PID and timestamp.
pub fn write_status_to(path: &Path, port: u16) -> Result<()> {
    let status = StatusFile {
        pid: std::process::id(),
        port,
        started_at: Some(chrono::Utc::now().to_rfc3339()),
        version: Some(env!("CARGO_PKG_VERSION").to_string()),
    };
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, serde_json::to_string_pretty(&status)?)?;
    Ok(())
}

/// Read and deserialize a status file from the given path.
///
/// Returns `None` if the file doesn't exist or contains invalid JSON.
pub fn read_status_from(path: &Path) -> Option<StatusFile> {
    let content = std::fs::read_to_string(path).ok()?;
    serde_json::from_str(&content).ok()
}

/// Remove a status file at the given path, ignoring errors.
pub fn remove_status_from(path: &Path) {
    let _ = std::fs::remove_file(path);
}

// --- Convenience wrappers (default path, used until Epic 4) ---

/// Write a status file at the default path.
pub fn write_status(port: u16) -> Result<()> {
    write_status_to(&get_default_status_path(), port)
}

/// Read the status file from the default path.
pub fn read_status() -> Option<StatusFile> {
    read_status_from(&get_default_status_path())
}

/// Remove the status file at the default path.
pub fn remove_status() {
    remove_status_from(&get_default_status_path());
}

// --- Running check with legacy fallback ---

/// Check if an adapter instance is running by reading the status file.
///
/// Checks the new `~/.copilot-adapter/status.json` first, then falls back
/// to legacy temp dir PID files for backward compatibility. Cleans up stale
/// files for dead processes.
pub fn is_running_from_status() -> Option<StatusFile> {
    // Check new location first
    if let Some(status) = read_status() {
        if super::process_exists(status.pid) {
            return Some(status);
        }
        // Stale status file — process is dead, clean up
        remove_status();
    }
    // Legacy fallback: check temp dir PID file
    let pid_path = super::get_pid_path();
    if let Ok(content) = std::fs::read_to_string(&pid_path) {
        if let Ok(pid) = content.trim().parse::<u32>() {
            if super::process_exists(pid) {
                let port = super::read_port().unwrap_or(0);
                return Some(StatusFile {
                    pid,
                    port,
                    started_at: None,
                    version: None,
                });
            }
            let _ = std::fs::remove_file(&pid_path);
        }
    }
    None
}
