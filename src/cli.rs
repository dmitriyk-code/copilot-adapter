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
    ///
    /// If not authenticated, will prompt for authentication in foreground mode.
    /// In daemon mode, authentication must be completed first.
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

        /// Cache TTL for the dynamic models list, in seconds (0 = no caching)
        #[arg(long, default_value_t = 300)]
        models_cache_ttl: u64,

        /// Always return the built-in static models list instead of fetching from Copilot API
        #[arg(long)]
        static_models: bool,

        /// Path to write human-readable conversation logs
        #[arg(long)]
        conversation_log: Option<String>,

        /// Maximum size for conversation log before rotation (bytes, default: 10MB)
        #[arg(long, default_value_t = 10_485_760)]
        conversation_log_max_size: u64,

        /// Enable verbose tool-related logging at INFO level
        #[arg(long)]
        debug_tools: bool,

        /// Skip automatic authentication if not logged in
        #[arg(long)]
        skip_auth: bool,

        /// Use native OpenAI function calling instead of XML prompt injection for tools.
        ///
        /// When enabled, tool definitions are forwarded natively to the Copilot API
        /// and tool call responses stream progressively. Falls back to XML injection
        /// if the upstream API does not support native tools.
        ///
        /// Cannot be used together with --xml-tools.
        #[arg(long, conflicts_with = "xml_tools")]
        native_tools: bool,

        /// Force XML-based tool injection (disables native tools).
        ///
        /// Tools are injected into the system prompt using XML format and parsed
        /// from the model's text response. This is the default behavior.
        /// Use this flag explicitly when you want to ensure XML mode is used,
        /// for example to override environment-level defaults or if native tools
        /// cause issues.
        ///
        /// Cannot be used together with --native-tools.
        #[arg(long, conflicts_with = "native_tools")]
        xml_tools: bool,

        /// Suppress startup guidance messages
        #[arg(short = 'q', long)]
        quiet: bool,
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
