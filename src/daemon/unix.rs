//! Unix-specific daemon operations using the `daemonize` crate.

use anyhow::{Context, Result};

use super::{is_running, remove_pid_file, remove_port_file};

/// Daemonize the current process using a double-fork pattern.
///
/// After this call, the parent process exits and the child continues
/// running in the background. The PID file is NOT written here — it is
/// written later by `server::run(write_pid=true)` to ensure a single
/// owner of the PID file across all platforms.
pub fn daemonize(log_file_path: Option<&str>) -> Result<()> {
    use daemonize::Daemonize;

    let mut daemon = Daemonize::new()
        .chown_pid_file(true)
        .working_directory(".");

    // Redirect stdout/stderr to log file if specified, otherwise /dev/null
    if let Some(path) = log_file_path {
        let stdout_file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .with_context(|| format!("Failed to open log file: {path}"))?;
        let stderr_file = stdout_file
            .try_clone()
            .context("Failed to clone log file handle")?;
        daemon = daemon.stdout(stdout_file).stderr(stderr_file);
    }

    daemon
        .start()
        .context("Failed to daemonize process")?;

    Ok(())
}

/// Stop the running daemon by sending SIGTERM.
///
/// Returns the PID of the stopped process on success.
pub fn stop_daemon() -> Result<u32> {
    if let Some(pid) = is_running() {
        // Send SIGTERM for graceful shutdown
        let ret = unsafe { libc::kill(pid as i32, libc::SIGTERM) };
        if ret != 0 {
            anyhow::bail!("Failed to send SIGTERM to process {pid}");
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
            anyhow::bail!("Process {pid} did not exit within timeout after SIGTERM");
        }

        remove_pid_file();
        remove_port_file();
        Ok(pid)
    } else {
        anyhow::bail!("Adapter is not running")
    }
}
