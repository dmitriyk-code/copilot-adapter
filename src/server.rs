use axum::{routing::get, Router};
use tower_http::cors::CorsLayer;
use tower_http::trace::TraceLayer;

use crate::handlers;

/// Build the axum Router with all routes and middleware layers.
pub fn build_router() -> Router {
    Router::new()
        // Health check — fully functional in Epic 1
        .route("/health", get(handlers::health::health))
        // Placeholder routes for future epics
        .route("/v1/chat/completions", axum::routing::post(placeholder))
        .route("/v1/models", get(placeholder))
        .route("/v1/models/{model}", get(placeholder))
        .layer(TraceLayer::new_for_http())
        .layer(CorsLayer::permissive())
}

/// Placeholder handler for routes to be implemented in later epics.
async fn placeholder() -> axum::http::StatusCode {
    axum::http::StatusCode::NOT_IMPLEMENTED
}

/// Start the HTTP server on the given host and port.
pub async fn run(host: &str, port: u16) -> anyhow::Result<()> {
    let app = build_router();
    let addr = format!("{host}:{port}");
    let listener = tokio::net::TcpListener::bind(&addr).await?;

    tracing::info!("Server listening on http://{addr}");

    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await?;

    Ok(())
}

/// Wait for CTRL+C (cross-platform graceful shutdown).
async fn shutdown_signal() {
    tokio::signal::ctrl_c()
        .await
        .expect("failed to install CTRL+C handler");
    tracing::info!("Shutdown signal received, stopping server");
}
