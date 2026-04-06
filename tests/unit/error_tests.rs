use axum::response::IntoResponse;
use copilot_adapter::error::AppError;
use regex;

/// Helper to convert an AppError to (StatusCode, JSON body).
async fn error_to_parts(error: AppError) -> (u16, serde_json::Value) {
    let response = error.into_response();
    let status = response.status().as_u16();
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    (status, json)
}

#[tokio::test]
async fn not_authenticated_returns_401_with_correct_format() {
    let (status, json) = error_to_parts(AppError::NotAuthenticated).await;
    assert_eq!(status, 401);
    assert_eq!(json["error"]["type"], "authentication_error");
    assert_eq!(json["error"]["code"], "not_authenticated");
    assert!(json["error"]["message"]
        .as_str()
        .unwrap()
        .contains("Authentication required"));
}

#[tokio::test]
async fn token_expired_returns_401_with_correct_format() {
    let (status, json) = error_to_parts(AppError::TokenExpired).await;
    assert_eq!(status, 401);
    assert_eq!(json["error"]["type"], "authentication_error");
    assert_eq!(json["error"]["code"], "token_expired");
    assert!(json["error"]["message"]
        .as_str()
        .unwrap()
        .contains("Token expired"));
}

#[tokio::test]
async fn github_error_returns_502_with_correct_format() {
    let (status, json) =
        error_to_parts(AppError::GitHubError("upstream failure".to_string())).await;
    assert_eq!(status, 502);
    assert_eq!(json["error"]["type"], "upstream_error");
    assert_eq!(json["error"]["code"], "github_error");
    assert_eq!(json["error"]["message"], "upstream failure");
}

#[tokio::test]
async fn copilot_error_returns_502_with_correct_format() {
    let (status, json) =
        error_to_parts(AppError::CopilotError("copilot failure".to_string())).await;
    assert_eq!(status, 502);
    assert_eq!(json["error"]["type"], "upstream_error");
    assert_eq!(json["error"]["code"], "copilot_error");
    assert_eq!(json["error"]["message"], "copilot failure");
}

#[tokio::test]
async fn rate_limited_returns_429_with_retry_after() {
    let error = AppError::RateLimited(30);
    let response = error.into_response();

    assert_eq!(response.status().as_u16(), 429);

    // Check Retry-After HTTP header
    let retry_after = response
        .headers()
        .get("Retry-After")
        .and_then(|v| v.to_str().ok())
        .unwrap();
    assert_eq!(retry_after, "30");

    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["error"]["type"], "rate_limit_error");
    assert_eq!(json["error"]["code"], "rate_limited");
    assert_eq!(json["error"]["retry_after"], 30);
    assert!(json["error"]["message"]
        .as_str()
        .unwrap()
        .contains("retry after 30s"));
}

#[tokio::test]
async fn invalid_request_returns_400_with_correct_format() {
    let (status, json) =
        error_to_parts(AppError::InvalidRequest("missing model".to_string())).await;
    assert_eq!(status, 400);
    assert_eq!(json["error"]["type"], "invalid_request_error");
    assert_eq!(json["error"]["code"], "invalid_request");
    assert_eq!(json["error"]["message"], "missing model");
}

#[tokio::test]
async fn model_not_found_returns_404_with_correct_format() {
    let (status, json) =
        error_to_parts(AppError::ModelNotFound("Model 'foo' not found".to_string())).await;
    assert_eq!(status, 404);
    assert_eq!(json["error"]["type"], "not_found_error");
    assert_eq!(json["error"]["code"], "model_not_found");
    assert_eq!(json["error"]["message"], "Model 'foo' not found");
}

#[tokio::test]
async fn internal_error_returns_500_with_correct_format() {
    let (status, json) = error_to_parts(AppError::Internal("something broke".to_string())).await;
    assert_eq!(status, 500);
    assert_eq!(json["error"]["type"], "internal_error");
    assert_eq!(json["error"]["code"], "internal_error");
    assert_eq!(json["error"]["message"], "something broke");
}

#[tokio::test]
async fn prompt_too_long_returns_400_with_correct_format() {
    let (status, json) = error_to_parts(AppError::PromptTooLong {
        actual_tokens: 168929,
        limit_tokens: 168000,
    })
    .await;
    assert_eq!(status, 400);
    assert_eq!(json["error"]["type"], "invalid_request_error");
    assert_eq!(json["error"]["code"], "prompt_too_long");
    assert_eq!(
        json["error"]["message"],
        "prompt is too long: 168929 tokens > 168000 maximum"
    );
}

#[tokio::test]
async fn prompt_too_long_message_matches_claude_code_regex() {
    let error = AppError::PromptTooLong {
        actual_tokens: 200000,
        limit_tokens: 128000,
    };
    let msg = error.to_string();
    // Claude Code's regex: /prompt is too long[^0-9]*(\d+)\s*tokens?\s*>\s*(\d+)/i
    let re = regex::Regex::new(r"(?i)prompt is too long[^0-9]*(\d+)\s*tokens?\s*>\s*(\d+)").unwrap();
    let caps = re.captures(&msg).expect("message must match Claude Code regex");
    assert_eq!(&caps[1], "200000");
    assert_eq!(&caps[2], "128000");
}

#[tokio::test]
async fn anyhow_error_converts_to_internal() {
    let anyhow_err = anyhow::anyhow!("unexpected failure");
    let app_err: AppError = anyhow_err.into();
    let (status, json) = error_to_parts(app_err).await;
    assert_eq!(status, 500);
    assert_eq!(json["error"]["type"], "internal_error");
    assert!(json["error"]["message"]
        .as_str()
        .unwrap()
        .contains("unexpected failure"));
}

#[test]
fn error_type_returns_correct_strings() {
    assert_eq!(
        AppError::NotAuthenticated.error_type(),
        "authentication_error"
    );
    assert_eq!(AppError::TokenExpired.error_type(), "authentication_error");
    assert_eq!(
        AppError::GitHubError("x".into()).error_type(),
        "upstream_error"
    );
    assert_eq!(
        AppError::CopilotError("x".into()).error_type(),
        "upstream_error"
    );
    assert_eq!(AppError::RateLimited(10).error_type(), "rate_limit_error");
    assert_eq!(
        AppError::InvalidRequest("x".into()).error_type(),
        "invalid_request_error"
    );
    assert_eq!(
        AppError::PromptTooLong {
            actual_tokens: 100,
            limit_tokens: 50
        }
        .error_type(),
        "invalid_request_error"
    );
    assert_eq!(
        AppError::ModelNotFound("x".into()).error_type(),
        "not_found_error"
    );
    assert_eq!(
        AppError::Internal("x".into()).error_type(),
        "internal_error"
    );
}

#[tokio::test]
async fn all_errors_share_openai_compatible_structure() {
    let errors: Vec<AppError> = vec![
        AppError::NotAuthenticated,
        AppError::TokenExpired,
        AppError::GitHubError("test".into()),
        AppError::CopilotError("test".into()),
        AppError::RateLimited(10),
        AppError::InvalidRequest("test".into()),
        AppError::PromptTooLong {
            actual_tokens: 100,
            limit_tokens: 50,
        },
        AppError::ModelNotFound("test".into()),
        AppError::Internal("test".into()),
    ];

    for error in errors {
        let response = error.into_response();
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();

        // Every error must have { "error": { "message": ..., "type": ..., "code": ... } }
        let error_obj = json.get("error").expect("must have 'error' field");
        assert!(
            error_obj.get("message").is_some(),
            "error must have 'message'"
        );
        assert!(error_obj.get("type").is_some(), "error must have 'type'");
        assert!(error_obj.get("code").is_some(), "error must have 'code'");
    }
}

// ---------------------------------------------------------------------------
// Epic 5 Task 5.1: Prompt-too-long error tests
// ---------------------------------------------------------------------------

/// Distinct from `prompt_too_long_returns_400_with_correct_format` (which uses 168929/168000).
/// Uses different token values and also verifies the Content-Type header is JSON.
#[tokio::test]
async fn prompt_too_long_returns_400_with_anthropic_format() {
    let error = AppError::PromptTooLong {
        actual_tokens: 250000,
        limit_tokens: 200000,
    };
    let response = error.into_response();
    assert_eq!(response.status().as_u16(), 400);

    // Verify Content-Type header
    let content_type = response
        .headers()
        .get("Content-Type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    assert!(
        content_type.contains("application/json"),
        "Content-Type should be application/json, got: {content_type}"
    );

    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["error"]["type"], "invalid_request_error");
    assert_eq!(json["error"]["code"], "prompt_too_long");
    let message = json["error"]["message"].as_str().unwrap();
    assert_eq!(
        message,
        "prompt is too long: 250000 tokens > 200000 maximum"
    );
}

#[test]
fn prompt_too_long_message_matches_claude_code_regex_sdk_format() {
    let err = AppError::PromptTooLong {
        actual_tokens: 168929,
        limit_tokens: 168000,
    };
    let message = err.to_string();
    let body = serde_json::json!({
        "error": {
            "message": message,
            "type": "invalid_request_error",
            "code": "prompt_too_long"
        }
    });
    let sdk_message = format!("400 {}", serde_json::to_string(&body).unwrap());
    // Claude Code's regex from src/services/api/errors.ts:89-90
    let re = regex::Regex::new(
        r"(?i)prompt is too long[^0-9]*(\d+)\s*tokens?\s*>\s*(\d+)",
    )
    .unwrap();
    let caps = re
        .captures(&sdk_message)
        .expect("regex must match SDK message");
    assert_eq!(caps.get(1).unwrap().as_str(), "168929");
    assert_eq!(caps.get(2).unwrap().as_str(), "168000");
}

#[test]
fn prompt_too_long_error_type() {
    let err = AppError::PromptTooLong {
        actual_tokens: 100000,
        limit_tokens: 50000,
    };
    assert_eq!(err.error_type(), "invalid_request_error");
}
