use clap::Parser;
use copilot_adapter::auth::browser::open_url;
use copilot_adapter::auth::device_flow::DeviceFlowAuth;
use copilot_adapter::auth::input::wait_for_enter_or_timeout;
use copilot_adapter::auth::token::TokenManager;
use copilot_adapter::cli::{Cli, Command, ProfilesAction};
use copilot_adapter::daemon;
use copilot_adapter::daemon::status::{read_status_from, remove_status_from};
use copilot_adapter::guidance;
use copilot_adapter::profile::types::Profile;
use copilot_adapter::profile::ProfileManager;
use copilot_adapter::server;
use copilot_adapter::storage;
use std::path::Path;
use std::time::Duration;
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Command::Start {
            daemon: is_daemon,
            port,
            host,
            log_level,
            log_file,
            models_cache_ttl,
            static_models,
            conversation_log,
            conversation_log_max_size,
            debug_tools,
            skip_auth,
            quiet,
            disable_native_tools,
            use_keyring,
            profile: profile_name,
        } => {
            let pm = ProfileManager::new();
            let profile = pm.get(&profile_name)?;

            // Check port conflicts across all profiles
            pm.check_port_conflict(port, &profile.name)?;

            // Check if this profile's instance is already running
            if let Some(status) = read_status_from(&profile.status_path()) {
                if daemon::process_exists(status.pid) {
                    eprintln!(
                        "Profile '{}' is already running (PID {}).",
                        profile.name, status.pid
                    );
                    eprintln!(
                        "Use 'copilot-adapter stop --profile {}' to stop it first.",
                        profile.name
                    );
                    std::process::exit(1);
                }
                // Stale status file — clean up
                remove_status_from(&profile.status_path());
            }

            // Pre-start auth check. If the check produces a valid TokenManager
            // with a cached Copilot token, we keep it and reuse it for the server
            // to avoid a redundant token-exchange network call on startup.
            let pre_validated_manager: Option<std::sync::Arc<TokenManager>> = if !skip_auth {
                let store = storage::create_storage_for_profile(&profile, use_keyring);
                let has_token = store.get_github_token().is_ok();

                if !has_token {
                    // Auth check runs before daemonization — parent has terminal access.
                    eprintln!("No authentication credentials found.");
                    eprintln!("Starting authentication flow...\n");
                    run_auth_for_profile(&profile, false, use_keyring).await?;
                    // run_auth uses its own TokenManager; create a fresh one for the server later.
                    None
                } else {
                    // Token exists — verify it's actually valid.
                    let auth_client = DeviceFlowAuth::new();
                    let manager =
                        std::sync::Arc::new(TokenManager::new(store, auth_client).await?);

                    match manager.get_valid_token().await {
                        Ok(_) => Some(manager),
                        Err(e) => {
                            eprintln!("Stored token is invalid or expired: {e}");
                            // Re-auth runs before daemonization — parent has terminal access.
                            eprintln!("Starting re-authentication...\n");
                            run_auth_for_profile(&profile, true, use_keyring).await?;
                            None
                        }
                    }
                }
            } else {
                None
            };

            if is_daemon {
                #[cfg(unix)]
                {
                    // Print brief daemon guidance before daemonizing, while we
                    // still have access to the parent's terminal.
                    if !quiet {
                        guidance::display_daemon_guidance(&host, port, None);
                    }
                    // On Unix, daemonize via double-fork before starting the server.
                    // After daemonize() we are in the child process.
                    daemon::daemonize(log_file.as_deref())?;
                    init_tracing_to_file(&log_level, log_file.as_deref());
                }

                #[cfg(windows)]
                {
                    // On Windows, spawn a detached child process without --daemon.
                    // The parent prints a message and exits immediately.
                    let mut args = vec!["start".to_string()];
                    args.push("--port".to_string());
                    args.push(port.to_string());
                    args.push("--host".to_string());
                    args.push(host.clone());
                    args.push("--log-level".to_string());
                    args.push(log_level.clone());
                    if let Some(ref lf) = log_file {
                        args.push("--log-file".to_string());
                        args.push(lf.clone());
                    }
                    args.push("--models-cache-ttl".to_string());
                    args.push(models_cache_ttl.to_string());
                    if static_models {
                        args.push("--static-models".to_string());
                    }
                    if let Some(ref cl) = conversation_log {
                        args.push("--conversation-log".to_string());
                        args.push(cl.clone());
                    }
                    args.push("--conversation-log-max-size".to_string());
                    args.push(conversation_log_max_size.to_string());
                    if debug_tools {
                        args.push("--debug-tools".to_string());
                    }
                    if disable_native_tools {
                        args.push("--disable-native-tools".to_string());
                    }
                    if use_keyring {
                        args.push("--use-keyring".to_string());
                    }
                    // Forward the profile name to the child process
                    args.push("--profile".to_string());
                    args.push(profile_name.clone());
                    // Always pass --skip-auth and --quiet to the daemon child process.
                    // The parent has already validated credentials above; the child
                    // runs without a terminal (stdin/stdout/stderr are null) and
                    // must not attempt interactive auth or print guidance.
                    args.push("--skip-auth".to_string());
                    args.push("--quiet".to_string());

                    let pid = daemon::spawn_background(&args)?;
                    if !quiet {
                        guidance::display_daemon_guidance(&host, port, Some(pid));
                    } else {
                        println!("Adapter started in background (PID {pid})");
                    }
                    return Ok(());
                }
            } else {
                init_tracing_to_file(&log_level, log_file.as_deref());
            }

            // Reuse the pre-validated manager if available (avoids a redundant
            // Copilot token exchange). Otherwise create a fresh one.
            let manager = match pre_validated_manager {
                Some(m) => m,
                None => {
                    let store = storage::create_storage_for_profile(&profile, use_keyring);
                    let auth_client = DeviceFlowAuth::new();
                    std::sync::Arc::new(TokenManager::new(store, auth_client).await?)
                }
            };

            tracing::info!(
                profile = %profile.name,
                "Starting copilot-adapter on {host}:{port}"
            );

            let config = server::AdapterConfig {
                static_models,
                models_cache_ttl: std::time::Duration::from_secs(models_cache_ttl),
                conversation_log_path: conversation_log.map(std::path::PathBuf::from),
                conversation_log_max_size,
                debug_tools,
                native_tools: !disable_native_tools,
            };

            if static_models {
                tracing::info!("Static models mode is ENABLED (dynamic fetching disabled)");
            } else {
                tracing::info!("Models cache TTL: {}s", models_cache_ttl);
            }

            if debug_tools {
                tracing::info!("Debug tools mode is ENABLED (verbose tool logging at INFO level)");
            }

            if disable_native_tools {
                tracing::info!("Native tools DISABLED (using XML tool injection only)");
            } else {
                tracing::info!(
                    "Native tools mode is ENABLED (OpenAI function calling, XML fallback)"
                );
            }

            // Display post-start guidance in foreground mode (unless suppressed).
            // In daemon mode the guidance was already shown above (or the Unix
            // daemon child has no terminal).
            if !is_daemon && !quiet {
                guidance::display_post_start_guidance(&host, port);
            }

            // write_pid=true when running as daemon so stop/status can find us;
            // also true in foreground mode for consistency with status command.
            let status_path = Some(profile.status_path());
            server::run(&host, port, manager, true, config, status_path).await?;
        }
        Command::Stop {
            profile: profile_name,
            all,
        } => {
            let pm = ProfileManager::new();

            if all {
                if profile_name != "default" {
                    eprintln!("Warning: --profile is ignored when --all is specified");
                }
                let mut stopped_any = false;
                for p in pm.list() {
                    match stop_profile_instance(&p.status_path()) {
                        Ok(StopOutcome::Stopped(pid)) => {
                            println!("Stopped profile '{}' (was PID {pid}).", p.name);
                            stopped_any = true;
                        }
                        Ok(StopOutcome::NotRunning) => {
                            // Silently skip profiles that aren't running
                        }
                        Err(e) => {
                            eprintln!(
                                "Warning: could not stop profile '{}': {}",
                                p.name, e
                            );
                        }
                    }
                }
                if !stopped_any {
                    println!("No running profiles found.");
                }
            } else {
                let profile = pm.get(&profile_name)?;
                match stop_profile_instance(&profile.status_path()) {
                    Ok(StopOutcome::Stopped(pid)) => {
                        println!("Adapter stopped (was PID {pid}).");
                    }
                    Ok(StopOutcome::NotRunning) => {
                        eprintln!("Adapter is not running.");
                        std::process::exit(1);
                    }
                    Err(e) => {
                        eprintln!("{e}");
                        std::process::exit(1);
                    }
                }
            }
        }
        Command::Status {
            profile: profile_name,
            all,
        } => {
            let pm = ProfileManager::new();

            if all {
                if profile_name != "default" {
                    eprintln!("Warning: --profile is ignored when --all is specified");
                }
                let profiles = pm.list();
                if profiles.is_empty() {
                    println!("No profiles found.");
                } else {
                    let mut any_running = false;
                    for p in &profiles {
                        if let Some(status) = read_status_from(&p.status_path()) {
                            if daemon::process_exists(status.pid) {
                                any_running = true;
                                println!("Profile '{}': running", p.name);
                                println!("  PID:        {}", status.pid);
                                if status.port > 0 {
                                    println!("  Port:       {}", status.port);
                                }
                                if let Some(ref version) = status.version {
                                    println!("  Version:    {}", version);
                                }
                                if let Some(ref started_at) = status.started_at {
                                    println!("  Started at: {}", started_at);
                                }
                            } else {
                                // Stale status — clean up
                                remove_status_from(&p.status_path());
                            }
                        }
                    }
                    if !any_running {
                        println!("No running profiles.");
                    }
                }
            } else {
                let profile = pm.get(&profile_name)?;
                match read_status_from(&profile.status_path()) {
                    Some(status) if daemon::process_exists(status.pid) => {
                        println!("Adapter running on PID {}", status.pid);
                        if status.port > 0 {
                            println!("  Port:       {}", status.port);
                        }
                        if let Some(ref version) = status.version {
                            println!("  Version:    {}", version);
                        }
                        if let Some(ref started_at) = status.started_at {
                            println!("  Started at: {}", started_at);
                        }
                        if profile_name != "default" {
                            println!("  Profile:    {}", profile_name);
                        }
                    }
                    Some(_) => {
                        // Stale status — clean up
                        remove_status_from(&profile.status_path());
                        println!("Adapter is not running.");
                    }
                    None => {
                        println!("Adapter is not running.");
                    }
                }
            }
        }
        Command::Auth {
            force,
            use_keyring,
            profile: profile_name,
        } => {
            init_tracing("info");
            let pm = ProfileManager::new();
            let profile = pm.get(&profile_name)?;
            run_auth_for_profile(&profile, force, use_keyring).await?;
        }
        Command::Logout {
            profile: profile_name,
        } => {
            init_tracing("info");
            let pm = ProfileManager::new();
            let profile = pm.get(&profile_name)?;
            run_logout_for_profile(&profile).await?;
        }
        Command::Profiles { action } => {
            let pm = ProfileManager::new();
            match action {
                ProfilesAction::List => {
                    let profiles = pm.list();
                    if profiles.is_empty() {
                        println!("No profiles found.");
                    } else {
                        println!("Profiles:");
                        for p in &profiles {
                            let status =
                                if let Some(s) = read_status_from(&p.status_path()) {
                                    if daemon::process_exists(s.pid) {
                                        format!("running (PID {}, port {})", s.pid, s.port)
                                    } else {
                                        "stopped".to_string()
                                    }
                                } else {
                                    "stopped".to_string()
                                };
                            println!("  {} ({})", p.name, status);
                        }
                    }
                }
                ProfilesAction::Create { name } => match pm.create(&name) {
                    Ok(_) => println!("Profile '{}' created.", name),
                    Err(e) => {
                        eprintln!("Error: {e}");
                        std::process::exit(1);
                    }
                },
                ProfilesAction::Delete { name } => {
                    // Check if the profile is currently running
                    if let Ok(profile) = pm.get(&name) {
                        if let Some(status) = read_status_from(&profile.status_path()) {
                            if daemon::process_exists(status.pid) {
                                eprintln!(
                                    "Profile '{}' is currently running (PID {}). Stop it first.",
                                    name, status.pid
                                );
                                std::process::exit(1);
                            }
                        }
                    }
                    match pm.delete(&name) {
                        Ok(()) => println!("Profile '{}' deleted.", name),
                        Err(e) => {
                            eprintln!("Error: {e}");
                            std::process::exit(1);
                        }
                    }
                }
            }
        }
    }

    Ok(())
}

/// Run the GitHub OAuth device flow and store the resulting tokens for a profile.
async fn run_auth_for_profile(
    profile: &Profile,
    force: bool,
    use_keyring: bool,
) -> anyhow::Result<()> {
    let store = storage::create_storage_for_profile(profile, use_keyring);
    let auth_client = DeviceFlowAuth::new();
    let manager = TokenManager::new(store, auth_client).await?;

    // Skip if already authenticated (unless --force)
    if !force && manager.is_authenticated().await {
        // Verify the token still works by getting a Copilot token
        match manager.get_valid_token().await {
            Ok(_) => {
                println!("Already authenticated. Use --force to re-authenticate.");
                return Ok(());
            }
            Err(_) => {
                println!("Stored token is invalid, re-authenticating...");
            }
        }
    }

    let response = manager.auth_client().initiate().await?;

    println!();
    println!("  To authenticate, visit:");
    println!();
    println!("    {}", response.verification_uri);
    println!();
    println!("  And enter this code: {}", response.user_code);
    println!();

    // Offer to open the authorization URL in the user's default browser.
    // Prefer verification_uri_complete (pre-fills the user code) when available.
    let url_to_open = response
        .verification_uri_complete
        .as_deref()
        .unwrap_or(&response.verification_uri);

    print!("  Press Enter to open in browser (or wait to continue manually)... ");
    use std::io::Write;
    std::io::stdout().flush()?;

    let should_open = wait_for_enter_or_timeout(Duration::from_secs(10));

    if should_open {
        match open_url(url_to_open) {
            Ok(true) => println!("  Browser opened!"),
            Ok(false) => println!("  Could not open browser. Please open the URL manually."),
            Err(e) => println!("  Failed to open browser: {e}"),
        }
    } else {
        println!(); // Move past the prompt line after timeout
    }

    println!();
    println!("  Waiting for authorization...");

    let github_token = manager
        .auth_client()
        .poll_for_token(
            &response.device_code,
            response.interval,
            response.expires_in,
        )
        .await?;

    // Store the GitHub token
    manager.set_github_token(github_token).await?;

    // Verify by getting a Copilot token
    match manager.get_valid_token().await {
        Ok(_) => {
            println!();
            println!("  ✓ Authentication successful! Copilot token obtained.");
            println!("  Credentials stored securely.");
        }
        Err(e) => {
            println!();
            println!("  ✓ GitHub authentication successful.");
            println!("  ⚠ Could not obtain Copilot token: {e}");
            println!("  (This may indicate your account doesn't have Copilot access.)");
        }
    }

    Ok(())
}

/// Clear stored credentials for a profile.
///
/// Clears both file and keyring storage to ensure credentials are
/// fully removed regardless of which backend was used previously.
async fn run_logout_for_profile(profile: &Profile) -> anyhow::Result<()> {
    // Clear file-based storage for this profile
    let file_store = storage::create_storage_for_profile(profile, false);
    let auth_client = DeviceFlowAuth::new();
    let manager = TokenManager::new(file_store, auth_client).await?;
    manager.clear_tokens().await?;

    // Also attempt to clear keyring storage (best-effort)
    let keyring_store = storage::create_storage_for_profile(profile, true);
    let auth_client2 = DeviceFlowAuth::new();
    if let Ok(kr_manager) = TokenManager::new(keyring_store, auth_client2).await {
        let _ = kr_manager.clear_tokens().await;
    }

    println!("Logged out. Credentials removed for profile '{}'.", profile.name);

    Ok(())
}

/// Outcome of attempting to stop a profile instance.
enum StopOutcome {
    /// Process was successfully terminated; contains the former PID.
    Stopped(u32),
    /// No running instance was found (missing or stale status file).
    NotRunning,
}

/// Stop a running instance identified by its status file path.
///
/// Reads the PID from the status file, terminates the process, waits for
/// exit, and cleans up the status file. Returns [`StopOutcome::NotRunning`]
/// when there is no status file or the recorded process is already dead.
fn stop_profile_instance(status_path: &Path) -> anyhow::Result<StopOutcome> {
    let status = match read_status_from(status_path) {
        Some(s) => s,
        None => return Ok(StopOutcome::NotRunning),
    };

    if !daemon::process_exists(status.pid) {
        // Stale status file — clean up
        remove_status_from(status_path);
        return Ok(StopOutcome::NotRunning);
    }

    let pid = status.pid;

    #[cfg(unix)]
    {
        let ret = unsafe { libc::kill(pid as i32, libc::SIGTERM) };
        if ret != 0 {
            anyhow::bail!("Failed to send SIGTERM to process {pid}");
        }
    }

    #[cfg(windows)]
    {
        // On Windows, taskkill /F performs a hard kill (no graceful shutdown).
        // Unlike Unix SIGTERM, this does not allow the server's shutdown_signal()
        // handler to drain in-flight requests. The status file is cleaned up
        // externally below, so data corruption is not a concern, but in-flight
        // SSE streams may be truncated. A soft-stop mechanism (e.g., named pipe
        // or Windows event) could improve this in the future.
        let output = std::process::Command::new("taskkill")
            .args(["/PID", &pid.to_string(), "/F"])
            .output()
            .map_err(|e| anyhow::anyhow!("Failed to execute taskkill: {e}"))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            anyhow::bail!("Failed to terminate process {pid}: {stderr}");
        }
    }

    // Wait for the process to exit
    for _ in 0..20 {
        std::thread::sleep(std::time::Duration::from_millis(100));
        if !daemon::process_exists(pid) {
            break;
        }
    }

    if daemon::process_exists(pid) {
        anyhow::bail!("Process {pid} did not exit within timeout after termination");
    }

    remove_status_from(status_path);
    Ok(StopOutcome::Stopped(pid))
}

/// Initialize tracing to stderr only (no log file).
fn init_tracing(log_level: &str) {
    let filter = build_env_filter(log_level);

    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_target(false)
        .init();
}

/// Initialize tracing with optional file output.
///
/// When `log_file` is `Some`, logs are written to the specified file.
/// When `None`, logs are written to stderr.
fn init_tracing_to_file(log_level: &str, log_file: Option<&str>) {
    let filter = build_env_filter(log_level);

    match log_file {
        Some(path) => {
            let file = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(path)
                .unwrap_or_else(|e| {
                    eprintln!("Failed to open log file {path}: {e}");
                    std::process::exit(1);
                });

            tracing_subscriber::fmt()
                .with_env_filter(filter)
                .with_target(false)
                .with_ansi(false)
                .with_writer(file)
                .init();
        }
        None => {
            tracing_subscriber::fmt()
                .with_env_filter(filter)
                .with_target(false)
                .init();
        }
    }
}

/// Build an `EnvFilter` that respects both `--log-level` and `RUST_LOG`.
///
/// **Precedence:** If the user explicitly set `--log-level` to a non-default
/// value, that takes precedence. Otherwise, `RUST_LOG` is used if set.
/// Falls back to `"info"` when neither is provided.
fn build_env_filter(log_level: &str) -> EnvFilter {
    if log_level != "info" {
        // User explicitly specified a non-default log level — it wins.
        EnvFilter::new(log_level)
    } else if std::env::var("RUST_LOG").is_ok() {
        // RUST_LOG is set and log_level is at the default — use RUST_LOG.
        EnvFilter::from_default_env()
    } else {
        // Neither overridden — use the default.
        EnvFilter::new(log_level)
    }
}
