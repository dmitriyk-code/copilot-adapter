use std::sync::Arc;

use axum::extract::{Path, State};
use axum::Json;

use crate::copilot::types::{Model, ModelList};
use crate::error::AppError;
use crate::server::AppState;

/// Hardcoded model creation timestamps for the fallback list.
const CREATED_GPT4O: i64 = 1715367049;
const CREATED_GPT4_TURBO: i64 = 1712361441;
const CREATED_GPT4: i64 = 1686935002;
const CREATED_GPT35_TURBO: i64 = 1677649963;

/// Return a static fallback model list used when the Copilot API is unavailable
/// or when `--static-models` mode is enabled.
fn fallback_models() -> ModelList {
    ModelList {
        object: "list".to_string(),
        data: vec![
            Model {
                id: "gpt-4o".to_string(),
                object: "model".to_string(),
                created: CREATED_GPT4O,
                owned_by: "github-copilot".to_string(),
            },
            Model {
                id: "gpt-4".to_string(),
                object: "model".to_string(),
                created: CREATED_GPT4,
                owned_by: "github-copilot".to_string(),
            },
            Model {
                id: "gpt-4-turbo".to_string(),
                object: "model".to_string(),
                created: CREATED_GPT4_TURBO,
                owned_by: "github-copilot".to_string(),
            },
            Model {
                id: "gpt-3.5-turbo".to_string(),
                object: "model".to_string(),
                created: CREATED_GPT35_TURBO,
                owned_by: "github-copilot".to_string(),
            },
        ],
    }
}

/// Resolve the current model list by checking cache, fetching from API, or
/// falling back to the static list.
///
/// Data flow:
/// 1. If `--static-models` → return fallback immediately
/// 2. If cache hit → return cached
/// 3. Get token → fetch from Copilot API
/// 4. On success → cache + return
/// 5. On error → log warning + return fallback
async fn resolve_models(state: &AppState) -> ModelList {
    // Static mode: skip all dynamic fetching.
    if state.config.static_models {
        tracing::debug!("Static models mode enabled, returning fallback list");
        return fallback_models();
    }

    // Check cache first.
    if let Some(cached) = state.models_cache.get().await {
        tracing::debug!("Models cache hit");
        return cached;
    }

    tracing::debug!("Models cache miss, fetching from Copilot API");

    // Get a valid Copilot token.
    let token = match state.token_manager.get_valid_token().await {
        Ok(t) => t,
        Err(e) => {
            tracing::warn!(
                error = %e,
                "Failed to obtain token for models fetch, using fallback list"
            );
            return fallback_models();
        }
    };

    // Fetch from the Copilot API.
    match state.copilot_client.fetch_models(&token).await {
        Ok(models) => {
            tracing::debug!(count = models.data.len(), "Fetched models from Copilot API");
            state.models_cache.set(models.clone()).await;
            models
        }
        Err(e) => {
            tracing::warn!(
                error = %e,
                "Failed to fetch models from Copilot API, using fallback list"
            );
            fallback_models()
        }
    }
}

/// Handler for `GET /v1/models` — returns the list of available models.
///
/// Attempts to return dynamically fetched models from the Copilot API (with
/// caching). Falls back to a static list on error or when `--static-models`
/// is enabled.
pub async fn list_models(State(state): State<Arc<AppState>>) -> Json<ModelList> {
    Json(resolve_models(&state).await)
}

/// Handler for `GET /v1/models/:model` — returns details for a specific model.
pub async fn get_model(
    State(state): State<Arc<AppState>>,
    Path(model_id): Path<String>,
) -> Result<Json<Model>, AppError> {
    let models = resolve_models(&state).await;

    models
        .data
        .into_iter()
        .find(|m| m.id == model_id)
        .map(Json)
        .ok_or_else(|| AppError::ModelNotFound(format!("Model '{model_id}' not found")))
}
