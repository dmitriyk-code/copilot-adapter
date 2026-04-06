use std::net::SocketAddr;

use axum::extract::Request;
use axum::http::StatusCode;
use axum::routing::get;
use axum::{Json, Router};
use serde_json::json;
use tokio::net::TcpListener;

use copilot_adapter::copilot::client::CopilotClient;

/// Spawn a mock models endpoint that returns the given status and body.
/// Returns the full URL to the mock models endpoint.
async fn spawn_mock_models(status: StatusCode, body: serde_json::Value) -> (String, SocketAddr) {
    let app = Router::new().route(
        "/models",
        get(move || {
            let body = body.clone();
            async move { (status, Json(body)) }
        }),
    );

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });

    (format!("http://{addr}/models"), addr)
}

/// Spawn a mock that validates required Copilot headers before returning
/// the model list. Returns 400 if any required header is missing or wrong.
async fn spawn_mock_models_with_header_validation() -> String {
    let app = Router::new().route(
        "/models",
        get(|req: Request| async move {
            let headers = req.headers();

            // Validate Authorization
            let auth = headers
                .get("Authorization")
                .and_then(|v| v.to_str().ok())
                .unwrap_or("");
            if !auth.starts_with("Bearer ") {
                return (
                    StatusCode::UNAUTHORIZED,
                    Json(json!({"error": "missing Bearer token"})),
                );
            }

            // Validate Copilot-Integration-Id
            let integration_id = headers
                .get("Copilot-Integration-Id")
                .and_then(|v| v.to_str().ok())
                .unwrap_or("");
            if integration_id != "vscode-chat" {
                return (
                    StatusCode::BAD_REQUEST,
                    Json(json!({"error": "bad Copilot-Integration-Id"})),
                );
            }

            // Validate Editor-Version
            let editor_version = headers
                .get("Editor-Version")
                .and_then(|v| v.to_str().ok())
                .unwrap_or("");
            if editor_version != "vscode/1.85.0" {
                return (
                    StatusCode::BAD_REQUEST,
                    Json(json!({"error": "bad Editor-Version"})),
                );
            }

            // Validate Editor-Plugin-Version
            let plugin_version = headers
                .get("Editor-Plugin-Version")
                .and_then(|v| v.to_str().ok())
                .unwrap_or("");
            if plugin_version != "copilot-chat/0.12.0" {
                return (
                    StatusCode::BAD_REQUEST,
                    Json(json!({"error": "bad Editor-Plugin-Version"})),
                );
            }

            (
                StatusCode::OK,
                Json(json!({
                    "object": "list",
                    "data": [
                        {
                            "id": "gpt-4o",
                            "object": "model",
                            "created": 1700000000,
                            "owned_by": "openai"
                        }
                    ]
                })),
            )
        }),
    );

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });

    format!("http://{addr}/models")
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn fetch_models_success_returns_model_list() {
    let (url, _addr) = spawn_mock_models(
        StatusCode::OK,
        json!({
            "object": "list",
            "data": [
                {
                    "id": "gpt-4o",
                    "object": "model",
                    "created": 1700000000,
                    "owned_by": "openai"
                },
                {
                    "id": "claude-sonnet-4",
                    "object": "model",
                    "created": 1700000001,
                    "owned_by": "anthropic"
                }
            ]
        }),
    )
    .await;

    let client = CopilotClient::new(reqwest::Client::new()).with_models_url(url);

    let result = client.fetch_models("test-token").await;
    assert!(result.is_ok(), "expected Ok, got {:?}", result);

    let model_list = result.unwrap();
    assert_eq!(model_list.object, "list");
    assert_eq!(model_list.data.len(), 2);
    assert_eq!(model_list.data[0].id, "gpt-4o");
    assert_eq!(model_list.data[0].owned_by, "openai");
    assert_eq!(model_list.data[1].id, "claude-sonnet-4");
    assert_eq!(model_list.data[1].owned_by, "anthropic");
}

#[tokio::test]
async fn fetch_models_sends_correct_headers() {
    let url = spawn_mock_models_with_header_validation().await;

    let client = CopilotClient::new(reqwest::Client::new()).with_models_url(url);

    let result = client.fetch_models("test-token").await;
    assert!(
        result.is_ok(),
        "expected Ok (headers validated by mock), got {:?}",
        result
    );

    let model_list = result.unwrap();
    assert_eq!(model_list.data.len(), 1);
    assert_eq!(model_list.data[0].id, "gpt-4o");
}

#[tokio::test]
async fn fetch_models_404_returns_copilot_error() {
    let (url, _addr) =
        spawn_mock_models(StatusCode::NOT_FOUND, json!({"error": "not found"})).await;

    let client = CopilotClient::new(reqwest::Client::new()).with_models_url(url);

    let result = client.fetch_models("test-token").await;
    assert!(result.is_err());

    let err = result.unwrap_err();
    let err_msg = err.to_string();
    assert!(
        err_msg.contains("Copilot API"),
        "expected CopilotError mentioning API, got: {err_msg}"
    );
    assert!(
        err_msg.contains("404"),
        "expected error to mention 404 status, got: {err_msg}"
    );
}

#[tokio::test]
async fn fetch_models_401_returns_copilot_error() {
    let (url, _addr) =
        spawn_mock_models(StatusCode::UNAUTHORIZED, json!({"error": "unauthorized"})).await;

    let client = CopilotClient::new(reqwest::Client::new()).with_models_url(url);

    let result = client.fetch_models("bad-token").await;
    assert!(result.is_err());

    let err = result.unwrap_err();
    let err_msg = err.to_string();
    assert!(
        err_msg.contains("Copilot API"),
        "expected CopilotError, got: {err_msg}"
    );
    assert!(
        err_msg.contains("401"),
        "expected error to mention 401 status, got: {err_msg}"
    );
}

#[tokio::test]
async fn fetch_models_malformed_json_returns_parse_error() {
    // Serve raw invalid JSON via a custom handler
    let app = Router::new().route(
        "/models",
        get(|| async {
            (
                StatusCode::OK,
                [("content-type", "application/json")],
                "{not valid json!!!",
            )
        }),
    );

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });

    let url = format!("http://{addr}/models");
    let client = CopilotClient::new(reqwest::Client::new()).with_models_url(url);

    let result = client.fetch_models("test-token").await;
    assert!(result.is_err());

    let err = result.unwrap_err();
    let err_msg = err.to_string();
    assert!(
        err_msg.contains("parse") || err_msg.contains("Parse"),
        "expected parse error, got: {err_msg}"
    );
}

#[tokio::test]
async fn fetch_models_handles_extra_fields_gracefully() {
    // Copilot API may return extra fields like `vendor` and `name`
    let (url, _addr) = spawn_mock_models(
        StatusCode::OK,
        json!({
            "object": "list",
            "data": [
                {
                    "id": "gpt-4o",
                    "object": "model",
                    "created": 1700000000,
                    "owned_by": "openai",
                    "vendor": "openai",
                    "name": "GPT-4o"
                }
            ]
        }),
    )
    .await;

    let client = CopilotClient::new(reqwest::Client::new()).with_models_url(url);

    let result = client.fetch_models("test-token").await;
    assert!(
        result.is_ok(),
        "extra fields should be ignored, got {:?}",
        result
    );

    let model_list = result.unwrap();
    assert_eq!(model_list.data.len(), 1);
    assert_eq!(model_list.data[0].id, "gpt-4o");
}

#[tokio::test]
async fn fetch_models_429_returns_rate_limited() {
    let app = Router::new().route(
        "/models",
        get(|| async {
            let mut response = (
                StatusCode::TOO_MANY_REQUESTS,
                Json(json!({"error": "rate limited"})),
            )
                .into_response();
            response
                .headers_mut()
                .insert("Retry-After", axum::http::HeaderValue::from_static("30"));
            response
        }),
    );

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });

    let url = format!("http://{addr}/models");
    let client = CopilotClient::new(reqwest::Client::new()).with_models_url(url);

    let result = client.fetch_models("test-token").await;
    assert!(result.is_err());

    let err_msg = result.unwrap_err().to_string();
    assert!(
        err_msg.contains("Rate limited") || err_msg.contains("rate"),
        "expected rate limit error, got: {err_msg}"
    );
}

use axum::response::IntoResponse;

use copilot_adapter::copilot::client::parse_prompt_too_long;

// ---------------------------------------------------------------------------
// parse_prompt_too_long tests
// ---------------------------------------------------------------------------

#[test]
fn parse_prompt_too_long_standard_format() {
    let body = r#"{"error":{"message":"prompt token count of 168929 exceeds the limit of 168000","code":"model_max_prompt_tokens_exceeded"}}"#;
    let result = parse_prompt_too_long(body);
    assert_eq!(result, Some((168929, 168000)));
}

#[test]
fn parse_prompt_too_long_wrong_code() {
    let body = r#"{"error":{"message":"prompt token count of 168929 exceeds the limit of 168000","code":"some_other_error"}}"#;
    assert_eq!(parse_prompt_too_long(body), None);
}

#[test]
fn parse_prompt_too_long_invalid_json() {
    assert_eq!(parse_prompt_too_long("not json"), None);
}

#[test]
fn parse_prompt_too_long_missing_error_field() {
    let body = r#"{"message":"something"}"#;
    assert_eq!(parse_prompt_too_long(body), None);
}

#[test]
fn parse_prompt_too_long_missing_code_field() {
    let body = r#"{"error":{"message":"prompt token count of 100 exceeds the limit of 50"}}"#;
    assert_eq!(parse_prompt_too_long(body), None);
}

#[test]
fn parse_prompt_too_long_missing_message_field() {
    let body = r#"{"error":{"code":"model_max_prompt_tokens_exceeded"}}"#;
    assert_eq!(parse_prompt_too_long(body), None);
}

#[test]
fn parse_prompt_too_long_unparseable_numbers() {
    let body = r#"{"error":{"message":"prompt token count of abc exceeds the limit of 168000","code":"model_max_prompt_tokens_exceeded"}}"#;
    assert_eq!(parse_prompt_too_long(body), None);
}

#[test]
fn parse_prompt_too_long_unexpected_message_format() {
    let body = r#"{"error":{"message":"something totally different","code":"model_max_prompt_tokens_exceeded"}}"#;
    assert_eq!(parse_prompt_too_long(body), None);
}

#[test]
fn parse_prompt_too_long_empty_body() {
    assert_eq!(parse_prompt_too_long(""), None);
}

#[test]
fn parse_prompt_too_long_small_values() {
    let body = r#"{"error":{"message":"prompt token count of 100 exceeds the limit of 50","code":"model_max_prompt_tokens_exceeded"}}"#;
    assert_eq!(parse_prompt_too_long(body), Some((100, 50)));
}
