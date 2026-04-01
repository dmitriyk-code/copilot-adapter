use axum::extract::State;
use axum::Json;
use std::sync::Arc;

use crate::anthropic::types::{CountTokensRequest, CountTokensResponse};
use crate::error::AppError;
use crate::server::AppState;
use crate::token_counter;

/// Handler for POST /v1/messages/count_tokens
///
/// Counts the number of input tokens for the given request using tiktoken.
/// Does not validate the model name — model validation happens at `/v1/messages`.
pub async fn count_tokens(
    State(_state): State<Arc<AppState>>,
    Json(request): Json<CountTokensRequest>,
) -> Result<Json<CountTokensResponse>, AppError> {
    let input_tokens = token_counter::count_tokens(&request)
        .map_err(|e| AppError::Internal(format!("Token counting failed: {e}")))?;

    Ok(Json(CountTokensResponse { input_tokens }))
}
