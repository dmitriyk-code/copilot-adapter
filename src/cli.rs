use clap::{Parser, Subcommand};

#[derive(Parser, Debug)]
#[command(
    name = "copilot-adapter",
    about = "OpenAI-compatible proxy to GitHub Copilot API",
    version
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Subcommand, Debug)]
pub enum Command {
    /// Start the adapter server
    Start {
        /// Run as background process
        #[arg(short, long)]
        daemon: bool,

        /// Port to listen on
        #[arg(short, long, default_value_t = 6767)]
        port: u16,

        /// Host to bind to
        #[arg(long, default_value = "127.0.0.1")]
        host: String,

        /// Log level: error, warn, info, debug, trace
        #[arg(long, default_value = "info")]
        log_level: String,

        /// Log file path (default: stderr)
        #[arg(long)]
        log_file: Option<String>,

        /// Enable experimental tool/function calling support via prompt injection
        #[arg(long)]
        experimental_tools: bool,
    },

    /// Stop the background adapter
    Stop,

    /// Show adapter status
    Status,

    /// Authenticate with GitHub
    Auth {
        /// Force re-authentication
        #[arg(long)]
        force: bool,
    },

    /// Remove stored credentials
    Logout,
}
