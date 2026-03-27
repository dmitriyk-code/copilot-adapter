//! Cross-platform background process management.
//!
//! Provides PID file management, process lifecycle checks, and platform-specific
//! daemonization (Unix) or background spawning (Windows).

#[cfg(unix)]
mod unix;
#[cfg(windows)]
mod windows;

#[cfg(unix)]
pub use unix::{daemonize, stop_daemon};
#[cfg(windows)]
pub use windows::{spawn_background, stop_daemon};

use std::path::PathBuf;

const PID_FILE: &str = "copilot-adapter.pid";

/// Returns the platform-appropriate path for the PID file.
///
/// - Unix: `/tmp/copilot-adapter.pid`
/// - Windows: `%TEMP%\copilot-adapter.pid`
pub fn get_pid_path() -> PathBuf {
    let dir = std::env::temp_dir();
    dir.join(PID_FILE)
}

/// Checks whether the daemon process is currently running.
///
/// Returns `Some(pid)` if a PID file exists and the process is alive,
/// or `None` if not running (also cleans up stale PID files).
pub fn is_running() -> Option<u32> {
    let pid_path = get_pid_path();
    if !pid_path.exists() {
        return None;
    }

    let pid: u32 = std::fs::read_to_string(&pid_path)
        .ok()?
        .trim()
        .parse()
        .ok()?;

    if process_exists(pid) {
        Some(pid)
    } else {
        // Stale PID file — clean it up
        let _ = std::fs::remove_file(&pid_path);
        None
    }
}

/// Writes the current process PID to the PID file.
pub fn write_pid_file() -> anyhow::Result<()> {
    let pid = std::process::id();
    let pid_path = get_pid_path();
    std::fs::write(&pid_path, pid.to_string())?;
    Ok(())
}

/// Removes the PID file if it exists.
pub fn remove_pid_file() {
    let _ = std::fs::remove_file(get_pid_path());
}

/// Checks whether a process with the given PID exists.
#[cfg(unix)]
pub fn process_exists(pid: u32) -> bool {
    // kill(pid, 0) checks if the process exists without sending a signal
    unsafe { libc::kill(pid as i32, 0) == 0 }
}

/// Checks whether a process with the given PID exists.
#[cfg(windows)]
pub fn process_exists(pid: u32) -> bool {
    use std::process::Command;

    // Use tasklist to check if the specific PID exists
    Command::new("tasklist")
        .args(["/FI", &format!("PID eq {pid}"), "/NH", "/FO", "CSV"])
        .output()
        .map(|output| {
            let stdout = String::from_utf8_lossy(&output.stdout);
            // Match the quoted PID field in CSV output to avoid substring false
            // positives (e.g. PID 123 matching inside "1234").
            stdout.contains(&format!("\"{pid}\""))
        })
        .unwrap_or(false)
}

/// Reads the port number from the PID file's sibling port file, if available.
pub fn read_port() -> Option<u16> {
    let port_path = get_pid_path().with_extension("port");
    std::fs::read_to_string(&port_path)
        .ok()?
        .trim()
        .parse()
        .ok()
}

/// Writes the port number to a file alongside the PID file.
pub fn write_port_file(port: u16) -> anyhow::Result<()> {
    let port_path = get_pid_path().with_extension("port");
    std::fs::write(&port_path, port.to_string())?;
    Ok(())
}

/// Removes the port file if it exists.
pub fn remove_port_file() {
    let port_path = get_pid_path().with_extension("port");
    let _ = std::fs::remove_file(port_path);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn get_pid_path_returns_temp_dir() {
        let path = get_pid_path();
        assert!(path.parent().is_some());
        assert_eq!(path.file_name().unwrap(), PID_FILE);
    }

    #[test]
    fn process_exists_finds_current_process() {
        assert!(process_exists(std::process::id()));
    }

    #[test]
    fn process_exists_returns_false_for_bogus_pid() {
        assert!(!process_exists(99999999));
    }
}
