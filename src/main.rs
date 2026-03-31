use clap::Parser;
use copilot_adapter::auth::browser::open_url;
use copilot_adapter::auth::device_flow::DeviceFlowAuth;
use copilot_adapter::auth::input::wait_for_enter_or_timeout;
use copilot_adapter::auth::token::TokenManager;
use copilot_adapter::cli::{Cli, Command};
use copilot_adapter::daemon;
use copilot_adapter::guidance;
use copilot_adapter::server;
use copilot_adapter::storage;
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
        } => {
            // Check if another instance is already running
            if let Some(pid) = daemon::is_running() {
                eprintln!("Adapter is already running (PID {pid}).");
                eprintln!("Use 'copilot-adapter stop' to stop it first.");
                std::process::exit(1);
            }

            // Pre-start auth check. If the check produces a valid TokenManager
            // with a cached Copilot token, we keep it and reuse it for the server
            // to avoid a redundant token-exchange network call on startup.
            let pre_validated_manager: Option<std::sync::Arc<TokenManager>> = if !skip_auth {
                let store = storage::create_storage();
                let has_token = store.get_github_token().is_ok();

                if !has_token {
                    if is_daemon {
                        eprintln!("No authentication credentials found.");
                        eprintln!("Please run 'copilot-adapter auth' first, or use --skip-auth to bypass.");
                        std::process::exit(1);
                    }

                    // Foreground mode: offer to authenticate now
                    eprintln!("No authentication credentials found.");
                    eprintln!("Starting authentication flow...\n");
                    run_auth(false).await?;
                    // run_auth uses its own TokenManager; create a fresh one for the server later.
                    None
                } else {
                    // Token exists — verify it's actually valid.
                    // Note: tracing is not yet initialized at this point, so log
                    // messages from TokenManager::new() and storage will be silently
                    // dropped. This is intentional — we need to validate credentials
                    // before setting up the server (and thus before configuring logging).
                    let auth_client = DeviceFlowAuth::new();
                    let manager =
                        std::sync::Arc::new(TokenManager::new(store, auth_client).await?);

                    match manager.get_valid_token().await {
                        Ok(_) => Some(manager),
                        Err(e) => {
                            if is_daemon {
                                eprintln!("Stored token is invalid or expired: {e}");
                                eprintln!("Please run 'copilot-adapter auth --force' first, or use --skip-auth to bypass.");
                                std::process::exit(1);
                            }

                            eprintln!("Stored token is invalid or expired: {e}");
                            eprintln!("Starting re-authentication...\n");
                            run_auth(true).await?;
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
                    // After daemonize(), we are in the child process.
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
                    let store = storage::create_storage();
                    let auth_client = DeviceFlowAuth::new();
                    std::sync::Arc::new(TokenManager::new(store, auth_client).await?)
                }
            };

            tracing::info!("Starting copilot-adapter on {host}:{port}");

            let config = server::AdapterConfig {
                static_models,
                models_cache_ttl: std::time::Duration::from_secs(models_cache_ttl),
                conversation_log_path: conversation_log.map(std::path::PathBuf::from),
                conversation_log_max_size,
                debug_tools,
            };

            if static_models {
                tracing::info!("Static models mode is ENABLED (dynamic fetching disabled)");
            } else {
                tracing::info!("Models cache TTL: {}s", models_cache_ttl);
            }

            if debug_tools {
                tracing::info!("Debug tools mode is ENABLED (verbose tool logging at INFO level)");
            }

            // Display post-start guidance in foreground mode (unless suppressed).
            // In daemon mode the guidance was already shown above (or the Unix
            // daemon child has no terminal).
            if !is_daemon && !quiet {
                guidance::display_post_start_guidance(&host, port);
            }

            // write_pid=true when running as daemon so stop/status can find us;
            // also true in foreground mode for consistency with status command.
            server::run(&host, port, manager, true, config).await?;
        }
        Command::Stop => {
            match daemon::stop_daemon() {
                Ok(pid) => println!("Adapter stopped (was PID {pid})."),
                Err(e) => {
                    eprintln!("{e}");
                    std::process::exit(1);
                }
            }
        }
        Command::Status => {
            match daemon::is_running() {
                Some(pid) => {
                    let port_info = daemon::read_port()
                        .map(|p| format!(", port {p}"))
                        .unwrap_or_default();
                    println!("Adapter running on PID {pid}{port_info}");
                }
                None => {
                    println!("Adapter is not running.");
                }
            }
        }
        Command::Auth { force } => {
            init_tracing("info");
            run_auth(force).await?;
        }
        Command::Logout => {
            init_tracing("info");
            run_logout().await?;
        }
    }

    Ok(())
}

/// Run the GitHub OAuth device flow and store the resulting tokens.
async fn run_auth(force: bool) -> anyhow::Result<()> {
    let store = storage::create_storage();
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

/// Clear all stored credentials.
async fn run_logout() -> anyhow::Result<()> {
    let store = storage::create_storage();
    let auth_client = DeviceFlowAuth::new();
    let manager = TokenManager::new(store, auth_client).await?;

    manager.clear_tokens().await?;
    println!("Logged out. All stored credentials have been removed.");

    Ok(())
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
