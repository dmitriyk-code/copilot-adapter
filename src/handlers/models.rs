use axum::extract::Path;
use axum::Json;

use crate::copilot::types::{Model, ModelList};
use crate::error::AppError;

/// Hardcoded model creation timestamp (2023-06-16, roughly when GPT-4 became widely available).
const MODEL_CREATED: i64 = 1686935002;

/// Return the list of available Copilot models.
fn available_models() -> Vec<Model> {
    vec![
        Model {
            id: "gpt-4".to_string(),
            object: "model".to_string(),
            created: MODEL_CREATED,
            owned_by: "github-copilot".to_string(),
        },
        Model {
            id: "gpt-4o".to_string(),
            object: "model".to_string(),
            created: MODEL_CREATED,
            owned_by: "github-copilot".to_string(),
        },
        Model {
            id: "gpt-4-turbo".to_string(),
            object: "model".to_string(),
            created: MODEL_CREATED,
            owned_by: "github-copilot".to_string(),
        },
        Model {
            id: "gpt-3.5-turbo".to_string(),
            object: "model".to_string(),
            created: MODEL_CREATED,
            owned_by: "github-copilot".to_string(),
        },
        Model {
            id: "claude-3.5-sonnet".to_string(),
            object: "model".to_string(),
            created: MODEL_CREATED,
            owned_by: "github-copilot".to_string(),
        },
    ]
}

/// Handler for `GET /v1/models` — returns the list of available models.
pub async fn list_models() -> Json<ModelList> {
    Json(ModelList {
        object: "list".to_string(),
        data: available_models(),
    })
}

/// Handler for `GET /v1/models/:model` — returns details for a specific model.
pub async fn get_model(Path(model_id): Path<String>) -> Result<Json<Model>, AppError> {
    available_models()
        .into_iter()
        .find(|m| m.id == model_id)
        .map(Json)
        .ok_or_else(|| {
            AppError::ModelNotFound(format!("Model '{model_id}' not found"))
        })
}
