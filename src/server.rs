use std::sync::Arc;
use std::time::Instant;

use axum::body::Body;
use axum::http::Request;
use axum::middleware;
use axum::response::Response;
use axum::{routing::get, Router};
use tower_http::cors::CorsLayer;

use crate::auth::token::TokenManager;
use crate::conversation_log::ConversationLogger;
use crate::copilot::client::CopilotClient;
use crate::copilot::models_cache::ModelsCache;
use crate::handlers;

/// Configuration options that control adapter behaviour.
#[derive(Debug, Clone)]
pub struct AdapterConfig {
    /// When `true`, `/v1/models` always returns the static fallback list
    /// without attempting to fetch from the Copilot API.
    pub static_models: bool,
    /// TTL for the dynamic models cache. After this duration the cached
    /// model list is considered stale and will be re-fetched.
    pub models_cache_ttl: std::time::Duration,
    /// Path to write human-readable conversation logs. `None` disables logging.
    pub conversation_log_path: Option<std::path::PathBuf>,
    /// Maximum size for the conversation log before rotation (bytes).
    pub conversation_log_max_size: u64,
    /// When `true`, emit additional INFO-level logs for tool injection/parsing.
    pub debug_tools: bool,
    /// When `true`, use native OpenAI function calling instead of XML prompt
    /// injection. Falls back to XML injection if the upstream API returns a
    /// tools-not-supported error. This is now the default behaviour.
    pub native_tools: bool,
}

impl Default for AdapterConfig {
    fn default() -> Self {
        Self {
            static_models: false,
            models_cache_ttl: std::time::Duration::from_secs(300),
            conversation_log_path: None,
            conversation_log_max_size: 10_485_760,
            debug_tools: false,
            native_tools: true,
        }
    }
}

/// Shared application state available to all handlers via axum's `State` extractor.
pub struct AppState {
    pub token_manager: Arc<TokenManager>,
    pub copilot_client: CopilotClient,
    pub config: AdapterConfig,
    /// In-memory cache for the Copilot models list with TTL-based expiration.
    pub models_cache: ModelsCache,
    /// Optional conversation logger for human-readable request/response logs.
    pub conversation_logger: Option<ConversationLogger>,
}

/// Request tracing middleware that logs method, path, status, duration, and request ID.
async fn request_tracing(req: Request<Body>, next: middleware::Next) -> Response {
    let request_id = uuid::Uuid::new_v4().to_string();
    let method = req.method().clone();
    let path = req.uri().path().to_string();
    let start = Instant::now();

    tracing::info!(
        request_id = %request_id,
        method = %method,
        path = %path,
        "Request received"
    );

    let mut response = next.run(req).await;

    let duration = start.elapsed();
    let status = response.status();

    tracing::info!(
        request_id = %request_id,
        method = %method,
        path = %path,
        status = status.as_u16(),
        duration_ms = duration.as_millis() as u64,
        "Request completed"
    );

    // Attach request ID as a response header for client-side debugging.
    response.headers_mut().insert(
        "X-Request-Id",
        axum::http::HeaderValue::from_str(&request_id)
            .unwrap_or_else(|_| axum::http::HeaderValue::from_static("unknown")),
    );

    response
}

/// Build the axum Router with all routes and middleware layers.
pub fn build_router(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/health", get(handlers::health::health))
        .route(
            "/v1/messages",
            axum::routing::post(handlers::messages::messages),
        )
        .route("/v1/models", get(handlers::models::list_models))
        .route("/v1/models/:model", get(handlers::models::get_model))
        .with_state(state)
        .layer(middleware::from_fn(request_tracing))
        .layer(CorsLayer::permissive())
}

/// Start the HTTP server on the given host and port.
///
/// When `write_pid` is true, writes PID and port files for daemon management.
/// On shutdown, PID and port files are cleaned up automatically.
pub async fn run(
    host: &str,
    port: u16,
    token_manager: Arc<TokenManager>,
    write_pid: bool,
    config: AdapterConfig,
) -> anyhow::Result<()> {
    let http_client = reqwest::Client::new();
    let models_cache = ModelsCache::new(config.models_cache_ttl);

    let conversation_logger = if let Some(ref path) = config.conversation_log_path {
        // Validate that the parent directory exists at startup
        if let Some(parent) = path.parent() {
            if !parent.as_os_str().is_empty() && !parent.exists() {
                anyhow::bail!(
                    "Conversation log directory does not exist: {}",
                    parent.display()
                );
            }
        }
        tracing::info!(
            path = %path.display(),
            max_size = config.conversation_log_max_size,
            "Conversation logging enabled"
        );
        Some(ConversationLogger::new(
            path,
            config.conversation_log_max_size,
        ))
    } else {
        None
    };

    let state = Arc::new(AppState {
        token_manager,
        copilot_client: CopilotClient::new(http_client),
        config,
        models_cache,
        conversation_logger,
    });

    let app = build_router(state);
    let addr = format!("{host}:{port}");
    let listener = tokio::net::TcpListener::bind(&addr).await?;

    if write_pid {
        crate::daemon::write_pid_file()?;
        crate::daemon::write_port_file(port)?;
    }

    tracing::info!("Server listening on http://{addr}");

    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await?;

    // Clean up PID/port files on graceful shutdown
    if write_pid {
        crate::daemon::remove_pid_file();
        crate::daemon::remove_port_file();
    }

    tracing::info!("Server stopped");
    Ok(())
}

/// Wait for a shutdown signal (SIGTERM/SIGINT on Unix, Ctrl+C on Windows).
///
/// On Unix, listens for both SIGTERM and SIGINT so that `stop_daemon()` (which
/// sends SIGTERM) triggers a graceful shutdown. On all platforms, Ctrl+C is handled.
async fn shutdown_signal() {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{signal, SignalKind};

        let mut sigterm =
            signal(SignalKind::terminate()).expect("failed to install SIGTERM handler");
        let mut sigint = signal(SignalKind::interrupt()).expect("failed to install SIGINT handler");

        tokio::select! {
            _ = sigterm.recv() => {
                tracing::info!("SIGTERM received, stopping server");
            }
            _ = sigint.recv() => {
                tracing::info!("SIGINT received, stopping server");
            }
        }
    }

    #[cfg(not(unix))]
    {
        tokio::signal::ctrl_c()
            .await
            .expect("failed to install CTRL+C handler");
        tracing::info!("Shutdown signal received, stopping server");
    }
}
