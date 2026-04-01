use std::process::Command;

/// Attempt to open a URL in the system's default browser.
///
/// Returns `Ok(true)` if the browser was launched successfully,
/// `Ok(false)` if no browser could be found, or `Err` on failure.
///
/// Uses platform-specific commands:
/// - Windows: `cmd /C start "" <url>`
/// - macOS: `open <url>`
/// - Linux: `xdg-open <url>` (falls back to `Ok(false)` if unavailable)
pub fn open_url(url: &str) -> anyhow::Result<bool> {
    #[cfg(target_os = "windows")]
    {
        Command::new("cmd").args(["/C", "start", "", url]).spawn()?;
        Ok(true)
    }

    #[cfg(target_os = "macos")]
    {
        Command::new("open").arg(url).spawn()?;
        return Ok(true);
    }

    #[cfg(target_os = "linux")]
    {
        if Command::new("xdg-open").arg(url).spawn().is_ok() {
            return Ok(true);
        }
        return Ok(false);
    }

    #[cfg(not(any(target_os = "windows", target_os = "macos", target_os = "linux")))]
    {
        Ok(false)
    }
}
