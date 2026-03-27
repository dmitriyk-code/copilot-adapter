use std::sync::Arc;

use axum::{routing::get, Router};
use tower_http::cors::CorsLayer;
use tower_http::trace::TraceLayer;

use crate::auth::token::TokenManager;
use crate::copilot::client::CopilotClient;
use crate::handlers;

/// Shared application state available to all handlers via axum's `State` extractor.
pub struct AppState {
    pub token_manager: Arc<TokenManager>,
    /// Shared HTTP client for direct upstream calls (used by the Epic 4 streaming handler).
    pub http_client: reqwest::Client,
    pub copilot_client: CopilotClient,
}

/// Build the axum Router with all routes and middleware layers.
pub fn build_router(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/health", get(handlers::health::health))
        .route(
            "/v1/chat/completions",
            axum::routing::post(handlers::chat::chat_completions),
        )
        .route("/v1/models", get(handlers::models::list_models))
        .route("/v1/models/:model", get(handlers::models::get_model))
        .with_state(state)
        .layer(TraceLayer::new_for_http())
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
) -> anyhow::Result<()> {
    let http_client = reqwest::Client::new();
    let state = Arc::new(AppState {
        token_manager,
        copilot_client: CopilotClient::new(http_client.clone()),
        http_client,
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
        let mut sigint =
            signal(SignalKind::interrupt()).expect("failed to install SIGINT handler");

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
