use clap::Parser;
use copilot_adapter::auth::device_flow::DeviceFlowAuth;
use copilot_adapter::auth::token::TokenManager;
use copilot_adapter::cli::{Cli, Command};
use copilot_adapter::daemon;
use copilot_adapter::server;
use copilot_adapter::storage;
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
        } => {
            // Check if another instance is already running
            if let Some(pid) = daemon::is_running() {
                eprintln!("Adapter is already running (PID {pid}).");
                eprintln!("Use 'copilot-adapter stop' to stop it first.");
                std::process::exit(1);
            }

            if is_daemon {
                #[cfg(unix)]
                {
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

                    let pid = daemon::spawn_background(&args)?;
                    println!("Adapter started in background (PID {pid})");
                    return Ok(());
                }
            } else {
                init_tracing_to_file(&log_level, log_file.as_deref());
            }

            let store = storage::create_storage();
            let auth_client = DeviceFlowAuth::new();
            let manager =
                std::sync::Arc::new(TokenManager::new(store, auth_client).await?);

            tracing::info!("Starting copilot-adapter on {host}:{port}");

            // write_pid=true when running as daemon so stop/status can find us;
            // also true in foreground mode for consistency with status command.
            server::run(&host, port, manager, true).await?;
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
    println!("  To authenticate, open the following URL in your browser:");
    println!();
    println!("    {}", response.verification_uri);
    println!();
    println!("  And enter this code: {}", response.user_code);
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
    let filter =
        EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(log_level));

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
    let filter =
        EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(log_level));

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
