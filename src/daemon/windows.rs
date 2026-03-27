//! Windows-specific background process management.
//!
//! On Windows we cannot use Unix daemonization, so we spawn a new detached
//! child process of the current executable (without the `--daemon` flag)
//! and let the parent exit immediately.

use anyhow::{Context, Result};
use std::os::windows::process::CommandExt;
use std::process::Command;

use super::{is_running, remove_pid_file, remove_port_file};

// Windows process creation flags
const CREATE_NO_WINDOW: u32 = 0x08000000;
const DETACHED_PROCESS: u32 = 0x00000008;

/// Spawn the adapter as a detached background process on Windows.
///
/// Re-launches the current executable with the same arguments minus `--daemon`/`-d`,
/// using Windows-specific process creation flags to detach from the console.
/// Returns the PID of the spawned child.
pub fn spawn_background(args: &[String]) -> Result<u32> {
    let exe = std::env::current_exe().context("Failed to get current executable path")?;

    // Filter out the --daemon / -d flag to prevent re-entry loop
    let filtered_args: Vec<&String> = args
        .iter()
        .filter(|a| *a != "--daemon" && *a != "-d")
        .collect();

    let child = Command::new(exe)
        .args(&filtered_args)
        .creation_flags(CREATE_NO_WINDOW | DETACHED_PROCESS)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .context("Failed to spawn background process")?;

    let pid = child.id();

    // Do NOT write the PID file here. The child process writes its own PID
    // via server::run(write_pid=true). Writing it here would cause the child
    // to find the PID file on startup and self-terminate with "already running".

    Ok(pid)
}

/// Stop the running daemon by terminating the process.
///
/// Returns the PID of the stopped process on success.
pub fn stop_daemon() -> Result<u32> {
    if let Some(pid) = is_running() {
        // Use taskkill for clean process termination
        let output = Command::new("taskkill")
            .args(["/PID", &pid.to_string(), "/F"])
            .output()
            .context("Failed to execute taskkill")?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            anyhow::bail!("Failed to terminate process {pid}: {stderr}");
        }

        // Wait briefly for the process to exit
        for _ in 0..20 {
            std::thread::sleep(std::time::Duration::from_millis(100));
            if !super::process_exists(pid) {
                break;
            }
        }

        // If the process is still alive after the timeout, report an error
        // and preserve the PID file so a follow-up stop attempt can retry.
        if super::process_exists(pid) {
            anyhow::bail!("Process {pid} did not exit within timeout after termination");
        }

        remove_pid_file();
        remove_port_file();
        Ok(pid)
    } else {
        anyhow::bail!("Adapter is not running")
    }
}
