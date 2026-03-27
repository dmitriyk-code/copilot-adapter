use clap::Parser;
use copilot_adapter::auth::device_flow::DeviceFlowAuth;
use copilot_adapter::auth::token::TokenManager;
use copilot_adapter::cli::{Cli, Command};
use copilot_adapter::server;
use copilot_adapter::storage;
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Command::Start {
            daemon: _daemon,
            port,
            host,
            log_level,
            log_file: _log_file,
        } => {
            init_tracing(&log_level);

            let store = storage::create_storage();
            let auth_client = DeviceFlowAuth::new();
            let manager =
                std::sync::Arc::new(TokenManager::new(store, auth_client).await?);

            tracing::info!("Starting copilot-adapter on {host}:{port}");

            // Daemon mode will be implemented in Epic 5
            server::run(&host, port, manager).await?;
        }
        Command::Stop => {
            // Will be implemented in Epic 5
            eprintln!("Stop command not yet implemented");
        }
        Command::Status => {
            // Will be implemented in Epic 5
            eprintln!("Status command not yet implemented");
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

fn init_tracing(log_level: &str) {
    let filter =
        EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(log_level));

    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_target(false)
        .init();
}
