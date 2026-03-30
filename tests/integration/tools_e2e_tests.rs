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
use copilot_adapter::auth::device_flow::DeviceFlowAuth;
use copilot_adapter::auth::token::TokenManager;
use copilot_adapter::copilot::client::CopilotClient;
use copilot_adapter::copilot::models_cache::ModelsCache;
use copilot_adapter::copilot::types::{ChatCompletionChunk, ChatCompletionResponse};
use copilot_adapter::server::{build_router, AdapterConfig, AppState};

use super::test_helpers::InMemoryStorage;

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

/// Create an `AppState` wired to the given mock servers.
async fn create_test_state(
    copilot_api_url: String,
    github_addr: std::net::SocketAddr,
) -> Arc<AppState> {
    let auth = DeviceFlowAuth::with_urls(
        format!("http://{github_addr}/unused"),
        format!("http://{github_addr}/unused"),
        format!("http://{github_addr}/copilot_internal/v2/token"),
    );

    let storage = InMemoryStorage::with_token("test_github_token");
    let tm = Arc::new(TokenManager::new(Box::new(storage), auth).await.unwrap());
    let client = reqwest::Client::new();

    Arc::new(AppState {
        token_manager: tm,
        copilot_client: CopilotClient::with_api_url(client.clone(), copilot_api_url),
        http_client: client,
        config: AdapterConfig::default(),
        models_cache: ModelsCache::new(std::time::Duration::from_secs(300)),
    })
}

/// Spawn a mock Copilot server backed by a custom handler function.
async fn spawn_mock_copilot_with_handler<F, Fut>(
    handler: F,
) -> (std::net::SocketAddr, tokio::task::JoinHandle<()>)
where
    F: Fn(axum::http::HeaderMap, Json<serde_json::Value>) -> Fut
        + Clone
        + Send
        + Sync
        + 'static,
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

/// Standard OpenAI tool definition for get_weather.
fn openai_weather_tool() -> serde_json::Value {
    json!({
        "type": "function",
        "function": {
            "name": "get_weather",
            "description": "Get the current weather in a given location",
            "parameters": {
                "type": "object",
                "properties": {
                    "location": {
                        "type": "string",
                        "description": "The city name"
                    }
                },
                "required": ["location"]
            }
        }
    })
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

/// Parse SSE body text into chunks and a done flag.
fn parse_sse_body(body_text: &str) -> (Vec<ChatCompletionChunk>, bool) {
    let mut chunks = Vec::new();
    let mut saw_done = false;

    for frame in body_text.split("\n\n") {
        let frame = frame.trim();
        if frame.is_empty() {
            continue;
        }
        for line in frame.lines() {
            let line = line.trim();
            if let Some(data) = line.strip_prefix("data:") {
                let data = data.trim();
                if data == "[DONE]" {
                    saw_done = true;
                } else if let Ok(chunk) = serde_json::from_str::<ChatCompletionChunk>(data) {
                    chunks.push(chunk);
                }
            }
        }
    }

    (chunks, saw_done)
}

// ===========================================================================
// E7-T2: Simple tool call (get_weather style)
// ===========================================================================

#[tokio::test]
async fn e2e_simple_tool_call_openai() {
    use crate::common::mock_copilot::build_tool_call_response;

    let (copilot_addr, _h1) = spawn_mock_copilot_with_handler(
        |_headers, Json(body): Json<serde_json::Value>| async move {
            let model = body["model"].as_str().unwrap_or("gpt-4");
            Json(build_tool_call_response(
                model,
                "get_weather",
                json!({"location": "London"}),
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
        "model": "gpt-4",
        "messages": [{"role": "user", "content": "What's the weather in London?"}],
        "tools": [openai_weather_tool()]
    });

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/chat/completions")
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
    let resp: ChatCompletionResponse = serde_json::from_slice(&bytes).unwrap();

    // Validate tool_calls
    let tool_calls = resp.choices[0]
        .message
        .tool_calls
        .as_ref()
        .expect("tool_calls should be present");
    assert_eq!(tool_calls.len(), 1);
    assert_eq!(tool_calls[0].function.name, Some("get_weather".to_string()));
    assert!(tool_calls[0].id.as_ref().unwrap().starts_with("call_"));
    assert_eq!(tool_calls[0].call_type, Some("function".to_string()));

    // Validate arguments
    let args: serde_json::Value =
        serde_json::from_str(tool_calls[0].function.arguments.as_ref().unwrap()).unwrap();
    assert_eq!(args["location"], "London");

    // Validate content is cleaned
    let content = resp.choices[0].message.content.as_text();
    assert!(!content.contains("<function_calls>"), "XML block should be stripped");
    assert!(
        !content.contains("<invoke"),
        "tool call XML should be stripped"
    );

    // Validate finish_reason
    assert_eq!(
        resp.choices[0].finish_reason,
        Some("tool_calls".to_string())
    );
}

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
async fn e2e_multi_turn_with_tool_results_openai() {
    use crate::common::mock_copilot::build_plain_response;

    // In the multi-turn scenario, the second request includes tool results.
    // The mock echoes back the messages to verify they were translated correctly.
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

    // Simulate multi-turn: user asked, assistant made a tool call, tool returned result
    let body = json!({
        "model": "gpt-4",
        "messages": [
            {"role": "user", "content": "What's the weather in London?"},
            {
                "role": "assistant",
                "content": "Let me check the weather.",
                "tool_calls": [{
                    "id": "call_abc123",
                    "type": "function",
                    "function": {
                        "name": "get_weather",
                        "arguments": "{\"location\":\"London\"}"
                    }
                }]
            },
            {
                "role": "tool",
                "content": "Sunny, 22°C, light breeze from the west",
                "tool_call_id": "call_abc123"
            }
        ],
        "tools": [openai_weather_tool()]
    });

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/chat/completions")
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
    let resp: ChatCompletionResponse = serde_json::from_slice(&bytes).unwrap();

    // The echo handler returns the messages sent to Copilot.
    // The tool message should be translated to user role with tool result content.
    let echoed_content = resp.choices[0].message.content.as_text();
    let echoed_messages: serde_json::Value =
        serde_json::from_str(&echoed_content).expect("echoed content should be valid JSON");

    // Verify the tool result was translated to a user message
    let translated = echoed_messages
        .as_array()
        .unwrap()
        .iter()
        .find(|m| {
            m["content"]
                .as_str()
                .map_or(false, |c| c.contains("Tool Result"))
        })
        .expect("Should find translated tool message");

    assert_eq!(translated["role"], "user");
    assert!(
        translated["content"]
            .as_str()
            .unwrap()
            .contains("call_abc123"),
        "Translated message should contain the tool_call_id"
    );
    assert!(
        translated["content"]
            .as_str()
            .unwrap()
            .contains("Sunny, 22°C"),
        "Translated message should contain the tool result"
    );
}

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
async fn e2e_complex_arguments_openai() {
    use crate::common::mock_copilot::build_tool_call_response;

    // XML parameters are flat key-value strings; use string-based args
    let complex_args = json!({
        "query": "SELECT * FROM users WHERE active = true",
        "limit": "100",
        "timeout_ms": "5000"
    });

    let (copilot_addr, _h1) = spawn_mock_copilot_with_handler({
        let args = complex_args.clone();
        move |_headers, Json(body): Json<serde_json::Value>| {
            let args = args.clone();
            async move {
                let model = body["model"].as_str().unwrap_or("gpt-4");
                Json(build_tool_call_response(
                    model,
                    "execute_query",
                    args,
                    Some(("Running your database query.", "Query submitted.")),
                ))
                .into_response()
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
        "model": "gpt-4",
        "messages": [{"role": "user", "content": "Run a complex query"}],
        "tools": [{
            "type": "function",
            "function": {
                "name": "execute_query",
                "description": "Execute a database query",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "query": {"type": "string"},
                        "limit": {"type": "string"},
                        "timeout_ms": {"type": "string"}
                    },
                    "required": ["query"]
                }
            }
        }]
    });

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/chat/completions")
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
    let resp: ChatCompletionResponse = serde_json::from_slice(&bytes).unwrap();

    let tool_calls = resp.choices[0]
        .message
        .tool_calls
        .as_ref()
        .expect("tool_calls should be present");
    assert_eq!(tool_calls.len(), 1);
    assert_eq!(
        tool_calls[0].function.name,
        Some("execute_query".to_string())
    );

    // Parse and verify arguments (all values are strings in XML)
    let args: serde_json::Value =
        serde_json::from_str(tool_calls[0].function.arguments.as_ref().unwrap()).unwrap();
    assert_eq!(args["query"], "SELECT * FROM users WHERE active = true");
    assert_eq!(args["limit"], "100");
    assert_eq!(args["timeout_ms"], "5000");
}

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
                Json(build_tool_call_response(model, "bash", args, None))
                    .into_response()
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
    assert_eq!(
        tool_use["input"]["working_directory"],
        "/home/user/project"
    );
    assert_eq!(tool_use["input"]["timeout"], "30");

    assert_eq!(resp.stop_reason, Some("tool_use".to_string()));
}

// ===========================================================================
// E7-T5: Response with no tool calls (graceful passthrough)
// ===========================================================================

#[tokio::test]
async fn e2e_no_tool_calls_passthrough_openai() {
    use crate::common::mock_copilot::build_plain_response;

    let (copilot_addr, _h1) = spawn_mock_copilot_with_handler(
        |_headers, Json(body): Json<serde_json::Value>| async move {
            let model = body["model"].as_str().unwrap_or("gpt-4");
            Json(build_plain_response(model, "The weather is nice today!")).into_response()
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
        "model": "gpt-4",
        "messages": [{"role": "user", "content": "What's the weather?"}],
        "tools": [openai_weather_tool()]
    });

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/chat/completions")
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
    let resp: ChatCompletionResponse = serde_json::from_slice(&bytes).unwrap();

    // No tool_calls should be present — model responded with plain text
    assert!(
        resp.choices[0].message.tool_calls.is_none(),
        "tool_calls should be None when model doesn't use tools"
    );

    // Content should be passed through intact
    let content = resp.choices[0].message.content.as_text();
    assert_eq!(content, "The weather is nice today!");

    // finish_reason should be "stop", not "tool_calls"
    assert_eq!(resp.choices[0].finish_reason, Some("stop".to_string()));
}

#[tokio::test]
async fn e2e_no_tool_calls_passthrough_anthropic() {
    use crate::common::mock_copilot::build_plain_response;

    let (copilot_addr, _h1) = spawn_mock_copilot_with_handler(
        |_headers, Json(body): Json<serde_json::Value>| async move {
            let model = body["model"].as_str().unwrap_or("gpt-4");
            Json(build_plain_response(model, "Here is a plain text response.")).into_response()
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
    assert!(!text_blocks.is_empty(), "should have at least one text block");
    assert_eq!(text_blocks[0]["text"], "Here is a plain text response.");

    // stop_reason should NOT be "tool_use"
    assert_ne!(resp.stop_reason, Some("tool_use".to_string()));
}

#[tokio::test]
async fn e2e_no_tool_calls_streaming_passthrough() {
    // The mock returns plain text (no tool calls embedded), and even though tools are in the
    // request, the adapter should pass through gracefully without injecting tool_calls into the response.
    let (copilot_addr, _h1) = spawn_mock_copilot_with_handler(
        |_headers, Json(body): Json<serde_json::Value>| async move {
            let model = body["model"].as_str().unwrap_or("gpt-4");

            // Return plain streaming content (no tool calls)
            let chunks = vec![
                format!(
                    "data: {}\n\n",
                    json!({
                        "id": "chatcmpl-e2e-stream",
                        "object": "chat.completion.chunk",
                        "created": 1700000000,
                        "model": model,
                        "choices": [{"index": 0, "delta": {"role": "assistant"}, "finish_reason": serde_json::Value::Null}]
                    })
                ),
                format!(
                    "data: {}\n\n",
                    json!({
                        "id": "chatcmpl-e2e-stream",
                        "object": "chat.completion.chunk",
                        "created": 1700000000,
                        "model": model,
                        "choices": [{"index": 0, "delta": {"content": "Just a normal response."}, "finish_reason": "stop"}]
                    })
                ),
                "data: [DONE]\n\n".to_string(),
            ];

            axum::http::Response::builder()
                .status(200)
                .header("Content-Type", "text/event-stream")
                .body(Body::from(chunks.concat()))
                .unwrap()
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
        "model": "gpt-4",
        "messages": [{"role": "user", "content": "Hello"}],
        "stream": true,
        "tools": [openai_weather_tool()]
    });

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/chat/completions")
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
    let body_text = String::from_utf8_lossy(&bytes);
    let (chunks, saw_done) = parse_sse_body(&body_text);

    assert!(saw_done, "Expected [DONE] marker");
    assert!(chunks.len() >= 2, "Expected at least 2 chunks");

    // No tool_calls in any chunk
    for chunk in &chunks {
        for choice in &chunk.choices {
            assert!(
                choice.delta.tool_calls.is_none(),
                "No tool_calls expected in passthrough streaming"
            );
        }
    }
}

// ===========================================================================
// E7-T6: Malformed tool call JSON in response
// ===========================================================================

#[tokio::test]
async fn e2e_malformed_tool_call_json_openai() {
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
        "model": "gpt-4",
        "messages": [{"role": "user", "content": "What's the weather?"}],
        "tools": [openai_weather_tool()]
    });

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/chat/completions")
                .header("Content-Type", "application/json")
                .body(Body::from(serde_json::to_vec(&body).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();

    // Should still succeed — malformed tool call is silently skipped
    assert_eq!(response.status(), StatusCode::OK);

    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let resp: ChatCompletionResponse = serde_json::from_slice(&bytes).unwrap();

    // No tool_calls should be parsed from malformed JSON
    assert!(
        resp.choices[0].message.tool_calls.is_none(),
        "Malformed tool call JSON should not produce tool_calls"
    );

    // Content should still be present (malformed block may or may not be stripped,
    // but the response should be valid)
    let content = resp.choices[0].message.content.as_text();
    assert!(
        !content.is_empty(),
        "Response content should not be empty"
    );

    // finish_reason should be "stop" since no valid tool calls were parsed
    assert_eq!(resp.choices[0].finish_reason, Some("stop".to_string()));
}

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

// ===========================================================================
// Additional E2E: streaming with tool call detection
// ===========================================================================

#[tokio::test]
async fn e2e_streaming_tool_call_openai() {
    use crate::common::mock_copilot::build_streaming_tool_call_chunks;

    let (copilot_addr, _h1) = spawn_mock_copilot_with_handler(
        |_headers, Json(body): Json<serde_json::Value>| async move {
            let model = body["model"].as_str().unwrap_or("gpt-4");
            let sse_body = build_streaming_tool_call_chunks(
                model,
                "get_weather",
                json!({"location": "Berlin"}),
            );

            axum::http::Response::builder()
                .status(200)
                .header("Content-Type", "text/event-stream")
                .body(Body::from(sse_body))
                .unwrap()
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
        "model": "gpt-4",
        "messages": [{"role": "user", "content": "What's the weather in Berlin?"}],
        "stream": true,
        "tools": [openai_weather_tool()]
    });

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/chat/completions")
                .header("Content-Type", "application/json")
                .body(Body::from(serde_json::to_vec(&body).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);

    let ct = response
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    assert!(ct.contains("text/event-stream"));

    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let body_text = String::from_utf8_lossy(&bytes);
    let (chunks, saw_done) = parse_sse_body(&body_text);

    assert!(saw_done, "Expected [DONE] marker");
    assert!(chunks.len() >= 2, "Expected at least 2 chunks");

    // Find the chunk with tool_calls
    let tool_chunk = chunks
        .iter()
        .find(|c| c.choices.iter().any(|ch| ch.delta.tool_calls.is_some()));

    assert!(tool_chunk.is_some(), "Expected a chunk with tool_calls");
    let tool_chunk = tool_chunk.unwrap();

    let tool_calls = tool_chunk.choices[0].delta.tool_calls.as_ref().unwrap();
    assert_eq!(tool_calls.len(), 1);
    assert_eq!(tool_calls[0].function.name, Some("get_weather".to_string()));

    let args: serde_json::Value =
        serde_json::from_str(tool_calls[0].function.arguments.as_ref().unwrap()).unwrap();
    assert_eq!(args["location"], "Berlin");

    // Text should not contain XML tool call markup
    let all_text: String = chunks
        .iter()
        .flat_map(|c| c.choices.iter())
        .filter_map(|ch| ch.delta.content.as_ref())
        .cloned()
        .collect();
    assert!(!all_text.contains("<function_calls>"));
    assert!(!all_text.contains("<invoke"));
}
