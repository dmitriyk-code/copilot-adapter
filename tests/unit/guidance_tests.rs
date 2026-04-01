use copilot_adapter::guidance;

// ─── Full guidance (foreground mode) ─────────────────────────

#[test]
fn full_guidance_contains_header_box() {
    let output = guidance::build_post_start_guidance("127.0.0.1", 6767);
    assert!(output.contains("Adapter Started Successfully"));
    assert!(output.contains("╔"));
    assert!(output.contains("╚"));
}

#[test]
fn full_guidance_contains_base_url() {
    let output = guidance::build_post_start_guidance("127.0.0.1", 6767);
    assert!(output.contains("http://127.0.0.1:6767"));
}

#[test]
fn full_guidance_contains_settings_json() {
    let output = guidance::build_post_start_guidance("127.0.0.1", 6767);
    assert!(output.contains("settings.json"));
    assert!(output.contains("ANTHROPIC_BASE_URL"));
    assert!(output.contains("ANTHROPIC_API_KEY"));
}

#[test]
fn full_guidance_custom_host_port() {
    let output = guidance::build_post_start_guidance("0.0.0.0", 9090);
    assert!(output.contains("http://0.0.0.0:9090"));
}

// ─── Windows guidance ────────────────────────────────────────

#[test]
fn windows_guidance_contains_powershell_syntax() {
    let output = guidance::build_windows_guidance("http://127.0.0.1:6767");
    assert!(output.contains("$env:ANTHROPIC_BASE_URL"));
    assert!(output.contains("$env:ANTHROPIC_API_KEY"));
    assert!(output.contains("PowerShell"));
}

#[test]
fn windows_guidance_contains_cmd_syntax() {
    let output = guidance::build_windows_guidance("http://127.0.0.1:6767");
    assert!(output.contains("set ANTHROPIC_BASE_URL=http://127.0.0.1:6767"));
    assert!(output.contains("set ANTHROPIC_API_KEY=dummy"));
    assert!(output.contains("CMD"));
}

#[test]
fn windows_guidance_contains_persist_instructions() {
    let output = guidance::build_windows_guidance("http://127.0.0.1:6767");
    assert!(output.contains("Environment Variables"));
    assert!(output.contains("System Properties"));
}

// ─── Unix guidance ───────────────────────────────────────────

#[test]
fn unix_guidance_zsh_shows_zshrc() {
    let output = guidance::build_unix_guidance_for_shell("http://127.0.0.1:6767", "/bin/zsh");
    assert!(output.contains("~/.zshrc"));
    assert!(output.contains("export ANTHROPIC_BASE_URL=http://127.0.0.1:6767"));
    assert!(output.contains("export ANTHROPIC_API_KEY=dummy"));
}

#[test]
fn unix_guidance_usr_bin_zsh_shows_zshrc() {
    let output = guidance::build_unix_guidance_for_shell("http://127.0.0.1:6767", "/usr/bin/zsh");
    assert!(output.contains("~/.zshrc"));
    assert!(output.contains("zsh"));
}

#[test]
fn unix_guidance_bash_shows_bashrc() {
    let output = guidance::build_unix_guidance_for_shell("http://127.0.0.1:6767", "/bin/bash");
    assert!(output.contains("~/.bashrc"));
    assert!(output.contains("export ANTHROPIC_BASE_URL=http://127.0.0.1:6767"));
}

#[test]
fn unix_guidance_unknown_shell_defaults_to_bash() {
    let output = guidance::build_unix_guidance_for_shell("http://127.0.0.1:6767", "/bin/fish");
    assert!(output.contains("~/.bashrc"));
}

#[test]
fn unix_guidance_empty_shell_defaults_to_bash() {
    let output = guidance::build_unix_guidance_for_shell("http://127.0.0.1:6767", "");
    assert!(output.contains("~/.bashrc"));
}

#[test]
fn unix_guidance_contains_current_session_option() {
    let output = guidance::build_unix_guidance_for_shell("http://127.0.0.1:6767", "/bin/bash");
    assert!(output.contains("Current session"));
}

#[test]
fn unix_guidance_contains_persist_option() {
    let output = guidance::build_unix_guidance_for_shell("http://127.0.0.1:6767", "/bin/bash");
    assert!(output.contains("Persist in bash"));
}

// ─── Settings JSON guidance ──────────────────────────────────

#[test]
fn settings_json_guidance_contains_json_block() {
    let output = guidance::build_settings_json_guidance("http://127.0.0.1:6767");
    assert!(output.contains(r#""ANTHROPIC_BASE_URL": "http://127.0.0.1:6767""#));
    assert!(output.contains(r#""ANTHROPIC_API_KEY": "dummy""#));
    assert!(output.contains(r#""env""#));
}

#[test]
fn settings_json_guidance_contains_recommended_label() {
    let output = guidance::build_settings_json_guidance("http://127.0.0.1:6767");
    assert!(output.contains("recommended"));
}

#[test]
fn settings_json_guidance_contains_precedence_info() {
    let output = guidance::build_settings_json_guidance("http://127.0.0.1:6767");
    assert!(output.contains("Settings precedence"));
    assert!(output.contains("settings.local.json"));
    assert!(output.contains("<project>/.claude/settings.json"));
    assert!(output.contains("~/.claude/settings.json"));
}

#[test]
fn settings_json_guidance_mentions_project_specific() {
    let output = guidance::build_settings_json_guidance("http://127.0.0.1:6767");
    assert!(output.contains("project-specific"));
}

// ─── Daemon guidance ─────────────────────────────────────────

#[test]
fn daemon_guidance_with_pid() {
    let output = guidance::build_daemon_guidance("127.0.0.1", 6767, Some(12345));
    assert!(output.contains("PID 12345"));
    assert!(output.contains("Configure Claude Code"));
    assert!(output.contains("settings.json"));
}

#[test]
fn daemon_guidance_without_pid() {
    let output = guidance::build_daemon_guidance("127.0.0.1", 6767, None);
    assert!(output.contains("background"));
    assert!(!output.contains("PID"));
    assert!(output.contains("Configure Claude Code"));
}

#[test]
fn daemon_guidance_contains_base_url() {
    let output = guidance::build_daemon_guidance("127.0.0.1", 6767, Some(1));
    assert!(output.contains("http://127.0.0.1:6767"));
}

#[test]
fn daemon_guidance_custom_host_port() {
    let output = guidance::build_daemon_guidance("0.0.0.0", 9090, Some(42));
    assert!(output.contains("http://0.0.0.0:9090"));
    assert!(output.contains("PID 42"));
}

#[test]
fn daemon_guidance_contains_env_vars() {
    let output = guidance::build_daemon_guidance("127.0.0.1", 6767, Some(1));
    assert!(output.contains("ANTHROPIC_BASE_URL"));
    assert!(output.contains("ANTHROPIC_API_KEY"));
}

// ─── Option numbering ───────────────────────────────────────

#[test]
fn settings_json_is_option_3() {
    let output = guidance::build_settings_json_guidance("http://127.0.0.1:6767");
    assert!(output.contains("Option 3"));
}

#[cfg(target_os = "windows")]
#[test]
fn full_guidance_windows_has_option_1_and_2() {
    let output = guidance::build_post_start_guidance("127.0.0.1", 6767);
    assert!(output.contains("Option 1: PowerShell"));
    assert!(output.contains("Option 2: CMD"));
    assert!(output.contains("Option 3"));
}

#[cfg(not(target_os = "windows"))]
#[test]
fn full_guidance_unix_has_option_1_and_2() {
    let output = guidance::build_post_start_guidance("127.0.0.1", 6767);
    assert!(output.contains("Option 1: Current session"));
    assert!(output.contains("Option 2: Persist in"));
    assert!(output.contains("Option 3"));
}
