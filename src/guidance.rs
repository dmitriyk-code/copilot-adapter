//! Post-start guidance display for new users.
//!
//! Shows copy-paste-able environment variable setup instructions and
//! settings.json guidance after the adapter starts, so new users can
//! immediately configure Claude Code without consulting the README.

use std::fmt::Write;

/// Display full post-start guidance to stdout (foreground mode).
pub fn display_post_start_guidance(host: &str, port: u16) {
    print!("{}", build_post_start_guidance(host, port));
}

/// Display brief daemon guidance to stdout (daemon mode).
///
/// If `pid` is `Some`, includes the PID in the message (available on Windows).
/// On Unix, the PID is not available before daemonization, so pass `None`.
pub fn display_daemon_guidance(host: &str, port: u16, pid: Option<u32>) {
    print!("{}", build_daemon_guidance(host, port, pid));
    // Flush to ensure output appears before the parent process exits
    // (especially important on Unix before daemonize()).
    use std::io::Write as IoWrite;
    let _ = std::io::stdout().flush();
}

/// Build the full post-start guidance text (foreground mode).
pub fn build_post_start_guidance(host: &str, port: u16) -> String {
    let base_url = format!("http://{}:{}", host, port);
    let mut out = String::new();

    // Header box
    writeln!(out).unwrap();
    writeln!(
        out,
        "╔══════════════════════════════════════════════════════════════╗"
    )
    .unwrap();
    writeln!(
        out,
        "║              Adapter Started Successfully                   ║"
    )
    .unwrap();
    writeln!(
        out,
        "╚══════════════════════════════════════════════════════════════╝"
    )
    .unwrap();
    writeln!(out).unwrap();
    writeln!(out, "Configure Claude Code to use this adapter:").unwrap();

    // Platform-specific environment variable guidance
    out.push_str(&build_platform_guidance(&base_url));

    // settings.json guidance (always shown)
    out.push_str(&build_settings_json_guidance(&base_url));

    out
}

/// Build brief daemon guidance text.
pub fn build_daemon_guidance(host: &str, port: u16, pid: Option<u32>) -> String {
    let base_url = format!("http://{}:{}", host, port);
    let mut out = String::new();

    match pid {
        Some(pid) => writeln!(out, "Adapter started in background (PID {pid})").unwrap(),
        None => writeln!(out, "Adapter starting in background...").unwrap(),
    }
    writeln!(out).unwrap();
    writeln!(out, "Configure Claude Code:").unwrap();

    // Show the most common env var syntax for the current platform
    #[cfg(target_os = "windows")]
    {
        writeln!(out, "  $env:ANTHROPIC_BASE_URL = \"{base_url}\"").unwrap();
        writeln!(out, "  $env:ANTHROPIC_API_KEY = \"dummy\"").unwrap();
    }
    #[cfg(not(target_os = "windows"))]
    {
        writeln!(out, "  export ANTHROPIC_BASE_URL={base_url}").unwrap();
        writeln!(out, "  export ANTHROPIC_API_KEY=dummy").unwrap();
    }

    writeln!(out).unwrap();
    writeln!(
        out,
        "Or add to ~/.claude/settings.json (see README for details)"
    )
    .unwrap();

    out
}

/// Dispatch to the correct platform-specific guidance builder.
#[cfg(target_os = "windows")]
fn build_platform_guidance(base_url: &str) -> String {
    build_windows_guidance(base_url)
}

/// Dispatch to the correct platform-specific guidance builder.
#[cfg(not(target_os = "windows"))]
fn build_platform_guidance(base_url: &str) -> String {
    let shell = detect_shell();
    build_unix_guidance_for_shell(base_url, &shell)
}

/// Build Windows-specific guidance (PowerShell + CMD).
///
/// Always compiled (not behind `cfg`) so it can be tested on any platform.
pub fn build_windows_guidance(base_url: &str) -> String {
    let mut out = String::new();

    writeln!(out).unwrap();
    writeln!(
        out,
        "── Option 1: PowerShell (current session) ─────────────────────"
    )
    .unwrap();
    writeln!(out).unwrap();
    writeln!(out, "  $env:ANTHROPIC_BASE_URL = \"{base_url}\"").unwrap();
    writeln!(out, "  $env:ANTHROPIC_API_KEY = \"dummy\"").unwrap();

    writeln!(out).unwrap();
    writeln!(
        out,
        "── Option 2: CMD (current session) ─────────────────────────────"
    )
    .unwrap();
    writeln!(out).unwrap();
    writeln!(out, "  set ANTHROPIC_BASE_URL={base_url}").unwrap();
    writeln!(out, "  set ANTHROPIC_API_KEY=dummy").unwrap();
    writeln!(out).unwrap();
    writeln!(out, "  To persist, add these via:").unwrap();
    writeln!(
        out,
        "    System Properties \u{2192} Advanced \u{2192} Environment Variables"
    )
    .unwrap();

    out
}

/// Build Unix-specific guidance using the `$SHELL` environment variable.
///
/// Always compiled (not behind `cfg`) so it can be tested on any platform.
pub fn build_unix_guidance(base_url: &str) -> String {
    let shell = detect_shell();
    build_unix_guidance_for_shell(base_url, &shell)
}

/// Build Unix-specific guidance for a given shell path.
///
/// Accepts an explicit shell string for testability (avoids env var races
/// when tests run in parallel).
pub fn build_unix_guidance_for_shell(base_url: &str, shell: &str) -> String {
    let (shell_name, rc_file) = parse_shell_rc(shell);
    let mut out = String::new();

    writeln!(out).unwrap();
    writeln!(
        out,
        "── Option 1: Current session ───────────────────────────────────"
    )
    .unwrap();
    writeln!(out).unwrap();
    writeln!(out, "  export ANTHROPIC_BASE_URL={base_url}").unwrap();
    writeln!(out, "  export ANTHROPIC_API_KEY=dummy").unwrap();

    writeln!(out).unwrap();
    writeln!(
        out,
        "── Option 2: Persist in {shell_name} ──────────────────────────────────"
    )
    .unwrap();
    writeln!(out).unwrap();
    writeln!(out, "  Add to {rc_file}:").unwrap();
    writeln!(out).unwrap();
    writeln!(out, "  export ANTHROPIC_BASE_URL={base_url}").unwrap();
    writeln!(out, "  export ANTHROPIC_API_KEY=dummy").unwrap();

    out
}

/// Build the settings.json guidance section (always shown, all platforms).
pub fn build_settings_json_guidance(base_url: &str) -> String {
    let mut out = String::new();

    writeln!(out).unwrap();
    writeln!(
        out,
        "── Option 3: Claude Code settings.json (recommended) ─────────"
    )
    .unwrap();
    writeln!(out).unwrap();
    writeln!(out, "  Create or edit ~/.claude/settings.json:").unwrap();
    writeln!(out).unwrap();
    writeln!(out, "  {{").unwrap();
    writeln!(out, "    \"env\": {{").unwrap();
    writeln!(out, "      \"ANTHROPIC_BASE_URL\": \"{base_url}\",").unwrap();
    writeln!(out, "      \"ANTHROPIC_API_KEY\": \"dummy\"").unwrap();
    writeln!(out, "    }}").unwrap();
    writeln!(out, "  }}").unwrap();
    writeln!(out).unwrap();
    writeln!(
        out,
        "  Also works as a project-specific .claude/settings.json"
    )
    .unwrap();
    writeln!(out).unwrap();
    writeln!(out, "  Settings precedence (highest first):").unwrap();
    writeln!(out, "    1. <project>/.claude/settings.local.json").unwrap();
    writeln!(out, "    2. <project>/.claude/settings.json").unwrap();
    writeln!(out, "    3. ~/.claude/settings.json").unwrap();
    writeln!(out).unwrap();

    out
}

/// Detect the user's shell from the `$SHELL` environment variable.
fn detect_shell() -> String {
    std::env::var("SHELL").unwrap_or_default()
}

/// Parse a shell path into a (name, rc_file) pair.
///
/// Returns `("zsh", "~/.zshrc")` if the shell path contains "zsh",
/// otherwise defaults to `("bash", "~/.bashrc")`.
fn parse_shell_rc(shell: &str) -> (&str, &str) {
    if shell.contains("zsh") {
        ("zsh", "~/.zshrc")
    } else {
        ("bash", "~/.bashrc")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_shell_rc_zsh() {
        assert_eq!(parse_shell_rc("/bin/zsh"), ("zsh", "~/.zshrc"));
        assert_eq!(parse_shell_rc("/usr/bin/zsh"), ("zsh", "~/.zshrc"));
    }

    #[test]
    fn parse_shell_rc_bash() {
        assert_eq!(parse_shell_rc("/bin/bash"), ("bash", "~/.bashrc"));
        assert_eq!(parse_shell_rc("/usr/bin/bash"), ("bash", "~/.bashrc"));
    }

    #[test]
    fn parse_shell_rc_unknown_defaults_to_bash() {
        assert_eq!(parse_shell_rc("/bin/fish"), ("bash", "~/.bashrc"));
        assert_eq!(parse_shell_rc(""), ("bash", "~/.bashrc"));
    }
}
