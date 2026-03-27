use clap::Parser;
use copilot_adapter::cli::{Cli, Command};
use copilot_adapter::server;
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
            tracing::info!("Starting copilot-adapter on {host}:{port}");

            // Daemon mode will be implemented in Epic 5
            server::run(&host, port).await?;
        }
        Command::Stop => {
            // Will be implemented in Epic 5
            eprintln!("Stop command not yet implemented");
        }
        Command::Status => {
            // Will be implemented in Epic 5
            eprintln!("Status command not yet implemented");
        }
        Command::Auth { force: _force } => {
            // Will be implemented in Epic 2
            eprintln!("Auth command not yet implemented");
        }
        Command::Logout => {
            // Will be implemented in Epic 2
            eprintln!("Logout command not yet implemented");
        }
    }

    Ok(())
}

fn init_tracing(log_level: &str) {
    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new(log_level));

    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_target(false)
        .init();
}
