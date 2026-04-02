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

        /// Disable native OpenAI function calling and force XML prompt injection for tools.
        ///
        /// By default, tool definitions are forwarded natively to the Copilot API
        /// and tool call responses stream progressively, with automatic fallback to
        /// XML injection if the upstream API does not support native tools.
        ///
        /// Use this flag to always use XML-based tool injection, where tools are
        /// injected into the system prompt using XML format and parsed from the
        /// model's text response. This may be useful if native tools cause issues.
        #[arg(long)]
        disable_native_tools: bool,

        /// Use the OS keyring for credential storage instead of the default file-based storage.
        ///
        /// By default, credentials are stored in ~/.copilot-adapter/credentials.json.
        /// With this flag, the adapter will try to use the OS keyring (macOS Keychain,
        /// Windows Credential Manager, or Linux Secret Service) and fall back to
        /// file storage if the keyring is unavailable.
        ///
        /// Note: Keyring entries are shared across profiles. Logging out with
        /// --use-keyring from any profile clears the shared keyring credential,
        /// affecting all profiles that use keyring storage.
        #[arg(long)]
        use_keyring: bool,

        /// Suppress startup guidance messages
        #[arg(short = 'q', long)]
        quiet: bool,

        /// Named profile to use (allows multiple concurrent instances)
        #[arg(short = 'P', long, default_value = "default")]
        profile: String,
    },

    /// Stop the background adapter
    Stop {
        /// Named profile to stop
        #[arg(short = 'P', long, default_value = "default")]
        profile: String,

        /// Stop all running profiles
        #[arg(long)]
        all: bool,
    },

    /// Show adapter status
    Status {
        /// Named profile to check
        #[arg(short = 'P', long, default_value = "default")]
        profile: String,

        /// Show status of all profiles
        #[arg(long)]
        all: bool,
    },

    /// Authenticate with GitHub
    Auth {
        /// Force re-authentication
        #[arg(long)]
        force: bool,

        /// Use the OS keyring for credential storage instead of the default file-based storage.
        /// Note: Keyring entries are shared across all profiles.
        #[arg(long)]
        use_keyring: bool,

        /// Named profile to authenticate
        #[arg(short = 'P', long, default_value = "default")]
        profile: String,
    },

    /// Remove stored credentials
    ///
    /// Clears stored credentials from both file storage and the OS keyring,
    /// regardless of which backend was used previously.
    Logout {
        /// Named profile to log out
        #[arg(short = 'P', long, default_value = "default")]
        profile: String,
    },

    /// Manage named profiles for multi-instance support
    Profiles {
        #[command(subcommand)]
        action: ProfilesAction,
    },
}

/// Subcommands for profile management.
#[derive(Subcommand, Debug)]
pub enum ProfilesAction {
    /// List all profiles
    List,

    /// Create a new named profile
    Create {
        /// Name of the profile to create
        name: String,
    },

    /// Delete an existing profile
    Delete {
        /// Name of the profile to delete
        name: String,
    },
}
