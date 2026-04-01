//! End-to-end integration tests for tool/function support.
//!
//! These tests spin up **both** a mock Copilot server and the full adapter
//! server stack, send real HTTP requests through the adapter, and assert on
//! the complete response including `tool_calls` structure.
//!
//! Covers:
//! - E7-T2: Simple tool call (get_weather style)
//! - E7-T3: Multi-turn conversation with tool results
//! - E7-T4: Tool call with complex arguments
//! - E7-T5: Response with no tool calls (graceful passthrough)
//! - E7-T6: Malformed tool call JSON in response

use std::sync::Arc;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::post;
use axum::{Json, Router};
use serde_json::json;
use tokio::net::TcpListener;
use tower::ServiceExt;

use copilot_adapter::anthropic::types::AnthropicResponse;
use copilot_adapter::server::build_router;

use super::test_helpers::create_test_state;

// ---------------------------------------------------------------------------
// Shared helpers
// ---------------------------------------------------------------------------

/// Spawn a mock GitHub server that provides Copilot tokens.
async fn spawn_mock_github() -> (std::net::SocketAddr, tokio::task::JoinHandle<()>) {
    let app = Router::new().route(
        "/copilot_internal/v2/token",
        axum::routing::get(|headers: axum::http::HeaderMap| async move {
            let auth = headers
                .get("Authorization")
                .and_then(|v| v.to_str().ok())
                .unwrap_or("");

            if auth == "token test_github_token" {
                let expires_at = chrono::Utc::now().timestamp() + 1800;
                Json(json!({
                    "token": "test_copilot_token",
                    "expires_at": expires_at
                }))
            } else {
                Json(json!({"error": "unauthorized"}))
            }
        }),
    );

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let handle = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });

    (addr, handle)
}

/// Spawn a mock Copilot server backed by a custom handler function.
async fn spawn_mock_copilot_with_handler<F, Fut>(
    handler: F,
) -> (std::net::SocketAddr, tokio::task::JoinHandle<()>)
where
    F: Fn(axum::http::HeaderMap, Json<serde_json::Value>) -> Fut + Clone + Send + Sync + 'static,
    Fut: std::future::Future<Output = Response> + Send,
{
    let app = Router::new().route(
        "/chat/completions",
        post(
            move |headers: axum::http::HeaderMap, body: Json<serde_json::Value>| {
                let h = handler.clone();
                async move { h(headers, body).await }
            },
        ),
    );

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let handle = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });

    (addr, handle)
}

/// Standard Anthropic tool definition for get_weather.
fn anthropic_weather_tool() -> serde_json::Value {
    json!({
        "name": "get_weather",
        "description": "Get the current weather in a given location",
        "input_schema": {
            "type": "object",
            "properties": {
                "location": {
                    "type": "string",
                    "description": "The city name"
                }
            },
            "required": ["location"]
        }
    })
}

// ===========================================================================
// E7-T2: Simple tool call (get_weather style)
// ===========================================================================

#[tokio::test]
async fn e2e_simple_tool_call_anthropic() {
    use crate::common::mock_copilot::build_tool_call_response;

    let (copilot_addr, _h1) = spawn_mock_copilot_with_handler(
        |_headers, Json(body): Json<serde_json::Value>| async move {
            let model = body["model"].as_str().unwrap_or("gpt-4");
            Json(build_tool_call_response(
                model,
                "get_weather",
                json!({"location": "Paris"}),
                None,
            ))
            .into_response()
        },
    )
    .await;

    let (github_addr, _h2) = spawn_mock_github().await;
    let state = create_test_state(
        format!("http://{copilot_addr}/chat/completions"),
        github_addr,
    )
    .await;
    let app = build_router(state);

    let body = json!({
        "model": "claude-sonnet-4-20250514",
        "max_tokens": 1024,
        "messages": [{"role": "user", "content": "What's the weather in Paris?"}],
        "tools": [anthropic_weather_tool()]
    });

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/messages")
                .header("Content-Type", "application/json")
                .body(Body::from(serde_json::to_vec(&body).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);

    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let resp: AnthropicResponse = serde_json::from_slice(&bytes).unwrap();

    let resp_json = serde_json::to_value(&resp).unwrap();
    let content = resp_json["content"].as_array().unwrap();

    // Should have a tool_use block
    let tool_use_blocks: Vec<_> = content.iter().filter(|b| b["type"] == "tool_use").collect();
    assert!(!tool_use_blocks.is_empty(), "should have tool_use block");

    let tool_use = &tool_use_blocks[0];
    assert_eq!(tool_use["name"], "get_weather");
    assert!(tool_use["id"].as_str().unwrap().starts_with("call_"));
    assert_eq!(tool_use["input"]["location"], "Paris");

    // stop_reason should be "tool_use"
    assert_eq!(resp.stop_reason, Some("tool_use".to_string()));
}

// ===========================================================================
// E7-T3: Multi-turn conversation with tool results
// ===========================================================================

#[tokio::test]
async fn e2e_multi_turn_with_tool_results_anthropic() {
    use crate::common::mock_copilot::build_plain_response;

    let (copilot_addr, _h1) = spawn_mock_copilot_with_handler(
        |_headers, Json(body): Json<serde_json::Value>| async move {
            let model = body["model"].as_str().unwrap_or("gpt-4");
            let messages = body["messages"].clone();
            let echo_content = serde_json::to_string_pretty(&messages).unwrap_or_default();
            Json(build_plain_response(model, &echo_content)).into_response()
        },
    )
    .await;

    let (github_addr, _h2) = spawn_mock_github().await;
    let state = create_test_state(
        format!("http://{copilot_addr}/chat/completions"),
        github_addr,
    )
    .await;
    let app = build_router(state);

    let body = json!({
        "model": "claude-sonnet-4-20250514",
        "max_tokens": 1024,
        "messages": [
            {"role": "user", "content": "What's the weather in Tokyo?"},
            {"role": "assistant", "content": [
                {"type": "text", "text": "Let me check."},
                {"type": "tool_use", "id": "call_xyz789", "name": "get_weather", "input": {"location": "Tokyo"}}
            ]},
            {"role": "user", "content": [
                {"type": "tool_result", "tool_use_id": "call_xyz789", "content": "Rainy, 15°C"}
            ]}
        ],
        "tools": [anthropic_weather_tool()]
    });

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/messages")
                .header("Content-Type", "application/json")
                .body(Body::from(serde_json::to_vec(&body).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);

    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let resp: AnthropicResponse = serde_json::from_slice(&bytes).unwrap();

    // The echo handler returns messages — verify tool_result was translated
    let echoed_content = resp.content[0].text_content();
    let echoed_messages: serde_json::Value =
        serde_json::from_str(echoed_content).expect("echoed content should be valid JSON");

    let translated = echoed_messages
        .as_array()
        .unwrap()
        .iter()
        .find(|m| {
            m["content"]
                .as_str()
                .map_or(false, |c| c.contains("Tool Result"))
        })
        .expect("Should find translated tool_result message");

    assert_eq!(translated["role"], "user");
    assert!(
        translated["content"]
            .as_str()
            .unwrap()
            .contains("call_xyz789"),
        "Translated message should contain the tool_use_id"
    );
    assert!(
        translated["content"]
            .as_str()
            .unwrap()
            .contains("Rainy, 15°C"),
        "Translated message should contain the tool result"
    );
}

// ===========================================================================
// E7-T4: Tool call with complex arguments
// ===========================================================================

#[tokio::test]
async fn e2e_complex_arguments_anthropic() {
    use crate::common::mock_copilot::build_tool_call_response;

    // XML parameters are flat key-value strings
    let complex_args = json!({
        "command": "find /home -name '*.rs' -type f",
        "working_directory": "/home/user/project",
        "timeout": "30"
    });

    let (copilot_addr, _h1) = spawn_mock_copilot_with_handler({
        let args = complex_args.clone();
        move |_headers, Json(body): Json<serde_json::Value>| {
            let args = args.clone();
            async move {
                let model = body["model"].as_str().unwrap_or("gpt-4");
                Json(build_tool_call_response(model, "bash", args, None)).into_response()
            }
        }
    })
    .await;

    let (github_addr, _h2) = spawn_mock_github().await;
    let state = create_test_state(
        format!("http://{copilot_addr}/chat/completions"),
        github_addr,
    )
    .await;
    let app = build_router(state);

    let body = json!({
        "model": "claude-sonnet-4-20250514",
        "max_tokens": 1024,
        "messages": [{"role": "user", "content": "Find all Rust files"}],
        "tools": [{
            "name": "bash",
            "description": "Execute a bash command",
            "input_schema": {
                "type": "object",
                "properties": {
                    "command": {"type": "string"},
                    "working_directory": {"type": "string"},
                    "timeout": {"type": "string"}
                },
                "required": ["command"]
            }
        }]
    });

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/messages")
                .header("Content-Type", "application/json")
                .body(Body::from(serde_json::to_vec(&body).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);

    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let resp: AnthropicResponse = serde_json::from_slice(&bytes).unwrap();

    let resp_json = serde_json::to_value(&resp).unwrap();
    let content = resp_json["content"].as_array().unwrap();

    let tool_use_blocks: Vec<_> = content.iter().filter(|b| b["type"] == "tool_use").collect();
    assert!(!tool_use_blocks.is_empty(), "should have tool_use block");

    let tool_use = &tool_use_blocks[0];
    assert_eq!(tool_use["name"], "bash");
    assert_eq!(
        tool_use["input"]["command"],
        "find /home -name '*.rs' -type f"
    );
    assert_eq!(tool_use["input"]["working_directory"], "/home/user/project");
    assert_eq!(tool_use["input"]["timeout"], "30");

    assert_eq!(resp.stop_reason, Some("tool_use".to_string()));
}

// ===========================================================================
// E7-T5: Response with no tool calls (graceful passthrough)
// ===========================================================================

#[tokio::test]
async fn e2e_no_tool_calls_passthrough_anthropic() {
    use crate::common::mock_copilot::build_plain_response;

    let (copilot_addr, _h1) = spawn_mock_copilot_with_handler(
        |_headers, Json(body): Json<serde_json::Value>| async move {
            let model = body["model"].as_str().unwrap_or("gpt-4");
            Json(build_plain_response(
                model,
                "Here is a plain text response.",
            ))
            .into_response()
        },
    )
    .await;

    let (github_addr, _h2) = spawn_mock_github().await;
    let state = create_test_state(
        format!("http://{copilot_addr}/chat/completions"),
        github_addr,
    )
    .await;
    let app = build_router(state);

    let body = json!({
        "model": "claude-sonnet-4-20250514",
        "max_tokens": 1024,
        "messages": [{"role": "user", "content": "Hello"}],
        "tools": [anthropic_weather_tool()]
    });

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/messages")
                .header("Content-Type", "application/json")
                .body(Body::from(serde_json::to_vec(&body).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);

    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let resp: AnthropicResponse = serde_json::from_slice(&bytes).unwrap();

    let resp_json = serde_json::to_value(&resp).unwrap();
    let content = resp_json["content"].as_array().unwrap();

    // Only text blocks, no tool_use blocks
    let tool_use_blocks: Vec<_> = content.iter().filter(|b| b["type"] == "tool_use").collect();
    assert!(tool_use_blocks.is_empty(), "should have no tool_use blocks");

    let text_blocks: Vec<_> = content.iter().filter(|b| b["type"] == "text").collect();
    assert!(
        !text_blocks.is_empty(),
        "should have at least one text block"
    );
    assert_eq!(text_blocks[0]["text"], "Here is a plain text response.");

    // stop_reason should NOT be "tool_use"
    assert_ne!(resp.stop_reason, Some("tool_use".to_string()));
}

// ===========================================================================
// E7-T6: Malformed tool call JSON in response
// ===========================================================================

#[tokio::test]
async fn e2e_malformed_tool_call_json_anthropic() {
    use crate::common::mock_copilot::build_malformed_tool_call_response;

    let (copilot_addr, _h1) = spawn_mock_copilot_with_handler(
        |_headers, Json(body): Json<serde_json::Value>| async move {
            let model = body["model"].as_str().unwrap_or("gpt-4");
            Json(build_malformed_tool_call_response(model)).into_response()
        },
    )
    .await;

    let (github_addr, _h2) = spawn_mock_github().await;
    let state = create_test_state(
        format!("http://{copilot_addr}/chat/completions"),
        github_addr,
    )
    .await;
    let app = build_router(state);

    let body = json!({
        "model": "claude-sonnet-4-20250514",
        "max_tokens": 1024,
        "messages": [{"role": "user", "content": "What's the weather?"}],
        "tools": [anthropic_weather_tool()]
    });

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/messages")
                .header("Content-Type", "application/json")
                .body(Body::from(serde_json::to_vec(&body).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();

    // Should succeed — graceful degradation
    assert_eq!(response.status(), StatusCode::OK);

    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let resp: AnthropicResponse = serde_json::from_slice(&bytes).unwrap();

    let resp_json = serde_json::to_value(&resp).unwrap();
    let content = resp_json["content"].as_array().unwrap();

    // No tool_use blocks — malformed JSON should be silently skipped
    let tool_use_blocks: Vec<_> = content.iter().filter(|b| b["type"] == "tool_use").collect();
    assert!(
        tool_use_blocks.is_empty(),
        "Malformed tool call should not produce tool_use blocks"
    );

    // stop_reason should NOT be "tool_use"
    assert_ne!(
        resp.stop_reason,
        Some("tool_use".to_string()),
        "stop_reason should not be 'tool_use' for malformed tool calls"
    );
}
