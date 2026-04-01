//! XML-specific integration tests for tool support.
//!
//! These tests verify the complete XML tool pipeline:
//! - Tool definitions are injected as valid XML into system prompts
//! - Attribute-based XML tool calls in responses are parsed correctly
//! - Tag-based XML tool calls in responses are parsed correctly
//! - Multiple tool calls work in a single response
//! - Streaming responses with XML tool calls work
//! - Standalone `<invoke>` blocks (without `<function_calls>` wrapper) work
//! - Special XML characters in parameters are handled

use std::sync::Arc;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::post;
use axum::{Json, Router};
use serde_json::json;
use tokio::net::TcpListener;
use tokio::sync::Mutex;
use tower::ServiceExt;

use copilot_adapter::anthropic::types::AnthropicResponse;
use copilot_adapter::server::build_router;

use super::test_helpers::create_test_state;

// ---------------------------------------------------------------------------
// Shared helpers
// ---------------------------------------------------------------------------

type CapturedBody = Arc<Mutex<Option<serde_json::Value>>>;

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

/// Spawn a capturing mock that stores the request body for inspection.
async fn spawn_capturing_mock() -> (
    std::net::SocketAddr,
    CapturedBody,
    tokio::task::JoinHandle<()>,
) {
    let captured: CapturedBody = Arc::new(Mutex::new(None));
    let captured_clone = captured.clone();

    let app = Router::new().route(
        "/chat/completions",
        post(move |Json(body): Json<serde_json::Value>| {
            let cap = captured_clone.clone();
            async move {
                let model = body["model"].as_str().unwrap_or("gpt-4").to_string();
                *cap.lock().await = Some(body);
                Json(json!({
                    "id": "chatcmpl-capture",
                    "object": "chat.completion",
                    "created": 1700000000,
                    "model": model,
                    "choices": [{
                        "index": 0,
                        "message": {"role": "assistant", "content": "OK"},
                        "finish_reason": "stop"
                    }],
                    "usage": {"prompt_tokens": 10, "completion_tokens": 1, "total_tokens": 11}
                }))
                .into_response()
            }
        }),
    );

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let handle = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    (addr, captured, handle)
}

fn sample_tool() -> serde_json::Value {
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

fn file_edit_tool() -> serde_json::Value {
    json!({
        "name": "edit_file",
        "description": "Edit a file at a given path",
        "input_schema": {
            "type": "object",
            "properties": {
                "path": {"type": "string", "description": "File path"},
                "content": {"type": "string", "description": "New file content"}
            },
            "required": ["path", "content"]
        }
    })
}

/// Parse Anthropic SSE events from raw response body text.
fn parse_sse_events(body_text: &str) -> Vec<serde_json::Value> {
    let mut events = Vec::new();
    for line in body_text.lines() {
        let line = line.trim();
        if let Some(data) = line.strip_prefix("data:") {
            let data = data.trim();
            if data.is_empty() || data == "[DONE]" {
                continue;
            }
            if let Ok(value) = serde_json::from_str::<serde_json::Value>(data) {
                events.push(value);
            }
        }
    }
    events
}

// ===========================================================================
// Test 1: XML tool injection produces valid XML in system prompt
// ===========================================================================

#[tokio::test]
async fn xml_tool_injection_produces_valid_xml_in_system_prompt() {
    let (copilot_addr, captured, _h1) = spawn_capturing_mock().await;
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
        "tools": [sample_tool(), file_edit_tool()]
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

    let captured_body = captured.lock().await;
    let body = captured_body
        .as_ref()
        .expect("mock should have captured body");
    let messages = body["messages"].as_array().unwrap();

    // First message should be system with injected tools
    assert_eq!(messages[0]["role"], "system");
    let system_content = messages[0]["content"].as_str().unwrap();

    // Verify XML structure
    assert!(
        system_content.contains("<tools>"),
        "System prompt should contain <tools> wrapper"
    );
    assert!(
        system_content.contains("</tools>"),
        "System prompt should contain closing </tools>"
    );
    assert!(
        system_content.contains("<tool_description>"),
        "Should have <tool_description> elements"
    );
    assert!(
        system_content.contains("<tool_name>get_weather</tool_name>"),
        "Should contain get_weather tool name"
    );
    assert!(
        system_content.contains("<tool_name>edit_file</tool_name>"),
        "Should contain edit_file tool name"
    );
    assert!(
        system_content.contains("<description>Get the current weather"),
        "Should contain tool description"
    );

    // Verify parameter definitions in XML
    assert!(
        system_content.contains("<name>location</name>"),
        "Should define location parameter"
    );
    assert!(
        system_content.contains("<required>true</required>"),
        "Should mark required parameters"
    );
    assert!(
        system_content.contains("<type>string</type>"),
        "Should include parameter types"
    );

    // Verify usage instructions are included
    assert!(
        system_content.contains("# Available Functions"),
        "Should have Available Functions heading"
    );
    assert!(
        system_content.contains("<function_calls>"),
        "Usage instructions should reference function_calls format"
    );
    assert!(
        system_content.contains(r#"invoke name="#),
        "Usage instructions should show attribute-based invoke format"
    );
    assert!(
        system_content.contains(r#"parameter name="#),
        "Usage instructions should show parameter format"
    );
}

// ===========================================================================
// Test 2: Attribute-based XML tool call parsed correctly
// ===========================================================================

#[tokio::test]
async fn xml_attribute_based_tool_call_parsed_correctly() {
    let (copilot_addr, _h1) = spawn_mock_copilot_with_handler(
        |_headers, Json(body): Json<serde_json::Value>| async move {
            let model = body["model"].as_str().unwrap_or("gpt-4");
            let content = r#"I'll check the weather.

<function_calls>
<invoke name="get_weather">
<parameter name="location">Tokyo</parameter>
</invoke>
</function_calls>"#;

            Json(json!({
                "id": "chatcmpl-attr-xml",
                "object": "chat.completion",
                "created": 1700000000,
                "model": model,
                "choices": [{
                    "index": 0,
                    "message": {"role": "assistant", "content": content},
                    "finish_reason": "stop"
                }],
                "usage": {"prompt_tokens": 10, "completion_tokens": 20, "total_tokens": 30}
            }))
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
        "messages": [{"role": "user", "content": "What's the weather in Tokyo?"}],
        "tools": [sample_tool()]
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
    assert_eq!(
        tool_use_blocks.len(),
        1,
        "Should have exactly one tool_use block"
    );

    let tool_use = &tool_use_blocks[0];
    assert_eq!(tool_use["name"], "get_weather");
    assert!(tool_use["id"].as_str().unwrap().starts_with("call_"));
    assert_eq!(tool_use["input"]["location"], "Tokyo");

    // Text content should have XML stripped
    let text_blocks: Vec<_> = content.iter().filter(|b| b["type"] == "text").collect();
    if !text_blocks.is_empty() {
        let text = text_blocks[0]["text"].as_str().unwrap();
        assert!(
            !text.contains("<function_calls>"),
            "XML should be stripped from text"
        );
        assert!(
            !text.contains("<invoke"),
            "Invoke tags should be stripped from text"
        );
    }

    assert_eq!(resp.stop_reason, Some("tool_use".to_string()));
}

// ===========================================================================
// Test 3: Tag-based XML tool call parsed correctly
// ===========================================================================

#[tokio::test]
async fn xml_tag_based_tool_call_parsed_correctly() {
    let (copilot_addr, _h1) = spawn_mock_copilot_with_handler(
        |_headers, Json(body): Json<serde_json::Value>| async move {
            let model = body["model"].as_str().unwrap_or("gpt-4");
            // Tag-based format (alternate dialect the parser supports)
            let content = r#"Let me read that file.

<function_calls>
<invoke>
<tool_name>read_file</tool_name>
<parameters>
<path>/src/main.rs</path>
</parameters>
</invoke>
</function_calls>"#;

            Json(json!({
                "id": "chatcmpl-tag-xml",
                "object": "chat.completion",
                "created": 1700000000,
                "model": model,
                "choices": [{
                    "index": 0,
                    "message": {"role": "assistant", "content": content},
                    "finish_reason": "stop"
                }],
                "usage": {"prompt_tokens": 10, "completion_tokens": 20, "total_tokens": 30}
            }))
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
        "messages": [{"role": "user", "content": "Read the file"}],
        "tools": [{
            "name": "read_file",
            "description": "Read a file",
            "input_schema": {
                "type": "object",
                "properties": {
                    "path": {"type": "string", "description": "File path"}
                },
                "required": ["path"]
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
    assert_eq!(
        tool_use_blocks.len(),
        1,
        "Should have exactly one tool_use block"
    );

    let tool_use = &tool_use_blocks[0];
    assert_eq!(tool_use["name"], "read_file");
    assert_eq!(tool_use["input"]["path"], "/src/main.rs");
    assert_eq!(resp.stop_reason, Some("tool_use".to_string()));
}

// ===========================================================================
// Test 4: Multiple XML tool calls in one response
// ===========================================================================

#[tokio::test]
async fn xml_multiple_tool_calls_in_single_response() {
    use crate::common::mock_copilot::build_multi_tool_call_response;

    let (copilot_addr, _h1) = spawn_mock_copilot_with_handler(
        |_headers, Json(body): Json<serde_json::Value>| async move {
            let model = body["model"].as_str().unwrap_or("gpt-4");
            Json(build_multi_tool_call_response(
                model,
                &[
                    ("read_file", json!({"path": "/src/main.rs"})),
                    ("get_weather", json!({"location": "Berlin"})),
                ],
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
        "messages": [{"role": "user", "content": "Read main.rs and check weather"}],
        "tools": [
            sample_tool(),
            {
                "name": "read_file",
                "description": "Read a file",
                "input_schema": {
                    "type": "object",
                    "properties": {
                        "path": {"type": "string", "description": "File path"}
                    },
                    "required": ["path"]
                }
            }
        ]
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
    assert_eq!(
        tool_use_blocks.len(),
        2,
        "Should have exactly two tool_use blocks"
    );

    // Each tool call should have a unique ID
    let id1 = tool_use_blocks[0]["id"].as_str().unwrap();
    let id2 = tool_use_blocks[1]["id"].as_str().unwrap();
    assert_ne!(id1, id2, "Tool call IDs should be unique");
    assert!(id1.starts_with("call_"));
    assert!(id2.starts_with("call_"));

    // Verify both tools are present (order may vary)
    let names: Vec<&str> = tool_use_blocks
        .iter()
        .map(|b| b["name"].as_str().unwrap())
        .collect();
    assert!(names.contains(&"read_file"), "Should contain read_file");
    assert!(names.contains(&"get_weather"), "Should contain get_weather");

    assert_eq!(resp.stop_reason, Some("tool_use".to_string()));
}

// ===========================================================================
// Test 5: Streaming with XML tool calls
// ===========================================================================

#[tokio::test]
async fn xml_streaming_tool_call_detected() {
    use crate::common::mock_copilot::build_streaming_tool_call_chunks;

    let (copilot_addr, _h1) = spawn_mock_copilot_with_handler(
        |_headers, Json(body): Json<serde_json::Value>| async move {
            let model = body["model"].as_str().unwrap_or("gpt-4");
            let sse_body = build_streaming_tool_call_chunks(
                model,
                "get_weather",
                json!({"location": "Paris"}),
            );

            axum::http::Response::builder()
                .status(200)
                .header("Content-Type", "text/event-stream")
                .body(Body::from(sse_body))
                .unwrap()
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
        "stream": true,
        "tools": [sample_tool()]
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
    let events = parse_sse_events(&body_text);

    // Should have message_start
    assert!(
        events.iter().any(|e| e["type"] == "message_start"),
        "Expected message_start event"
    );

    // Should have tool_use content block
    let tool_use_starts: Vec<_> = events
        .iter()
        .filter(|e| e["type"] == "content_block_start" && e["content_block"]["type"] == "tool_use")
        .collect();
    assert!(
        !tool_use_starts.is_empty(),
        "Expected tool_use content_block_start"
    );

    let tool_block = &tool_use_starts[0]["content_block"];
    assert_eq!(tool_block["name"], "get_weather");
    assert!(tool_block["id"].as_str().unwrap().starts_with("call_"));

    // Should have input_json_delta with the arguments
    let input_deltas: Vec<_> = events
        .iter()
        .filter(|e| e["type"] == "content_block_delta" && e["delta"]["type"] == "input_json_delta")
        .collect();
    assert!(!input_deltas.is_empty(), "Expected input_json_delta event");

    // Collect and concatenate all partial_json slices — the Anthropic SSE spec
    // allows `partial_json` to be fragmented across multiple delta events.
    let combined_json: String = input_deltas
        .iter()
        .filter_map(|e| e["delta"]["partial_json"].as_str())
        .collect();
    let input: serde_json::Value = serde_json::from_str(&combined_json).unwrap();
    assert_eq!(input["location"], "Paris");

    // Text deltas should not contain XML markup
    let text_deltas: Vec<_> = events
        .iter()
        .filter(|e| e["type"] == "content_block_delta" && e["delta"]["type"] == "text_delta")
        .collect();
    let all_text: String = text_deltas
        .iter()
        .filter_map(|e| e["delta"]["text"].as_str())
        .collect();
    assert!(
        !all_text.contains("<function_calls>"),
        "XML should be stripped from text deltas"
    );

    // message_delta should have stop_reason = "tool_use"
    let message_delta = events
        .iter()
        .find(|e| e["type"] == "message_delta")
        .expect("Expected message_delta event");
    assert_eq!(message_delta["delta"]["stop_reason"], "tool_use");

    // Should have message_stop
    assert!(
        events.iter().any(|e| e["type"] == "message_stop"),
        "Expected message_stop event"
    );
}

// ===========================================================================
// Test 6: Standalone invoke (no function_calls wrapper) — fallback parsing
// ===========================================================================

#[tokio::test]
async fn xml_standalone_invoke_parsed_as_fallback() {
    let (copilot_addr, _h1) = spawn_mock_copilot_with_handler(
        |_headers, Json(body): Json<serde_json::Value>| async move {
            let model = body["model"].as_str().unwrap_or("gpt-4");
            // No <function_calls> wrapper — just bare <invoke>
            let content = r#"Sure, I'll read that file.

<invoke name="read_file">
<parameter name="path">/tmp/test.txt</parameter>
</invoke>"#;

            Json(json!({
                "id": "chatcmpl-standalone",
                "object": "chat.completion",
                "created": 1700000000,
                "model": model,
                "choices": [{
                    "index": 0,
                    "message": {"role": "assistant", "content": content},
                    "finish_reason": "stop"
                }],
                "usage": {"prompt_tokens": 10, "completion_tokens": 15, "total_tokens": 25}
            }))
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
        "messages": [{"role": "user", "content": "Read the test file"}],
        "tools": [{
            "name": "read_file",
            "description": "Read a file",
            "input_schema": {
                "type": "object",
                "properties": {
                    "path": {"type": "string", "description": "File path"}
                },
                "required": ["path"]
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
    assert_eq!(
        tool_use_blocks.len(),
        1,
        "Standalone invoke should be parsed as fallback"
    );

    assert_eq!(tool_use_blocks[0]["name"], "read_file");
    assert_eq!(tool_use_blocks[0]["input"]["path"], "/tmp/test.txt");
    assert_eq!(resp.stop_reason, Some("tool_use".to_string()));
}

// ===========================================================================
// Test 7: Tool call with no parameters
// ===========================================================================

#[tokio::test]
async fn xml_tool_call_with_no_parameters() {
    let (copilot_addr, _h1) = spawn_mock_copilot_with_handler(
        |_headers, Json(body): Json<serde_json::Value>| async move {
            let model = body["model"].as_str().unwrap_or("gpt-4");
            let content = r#"I'll list the directory.

<function_calls>
<invoke name="list_dir">
</invoke>
</function_calls>"#;

            Json(json!({
                "id": "chatcmpl-noparam",
                "object": "chat.completion",
                "created": 1700000000,
                "model": model,
                "choices": [{
                    "index": 0,
                    "message": {"role": "assistant", "content": content},
                    "finish_reason": "stop"
                }],
                "usage": {"prompt_tokens": 10, "completion_tokens": 10, "total_tokens": 20}
            }))
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
        "messages": [{"role": "user", "content": "List the directory"}],
        "tools": [{
            "name": "list_dir",
            "description": "List directory contents",
            "input_schema": {
                "type": "object",
                "properties": {},
                "required": []
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
    assert_eq!(tool_use_blocks.len(), 1);
    assert_eq!(tool_use_blocks[0]["name"], "list_dir");

    // Input should be an empty object
    let input = &tool_use_blocks[0]["input"];
    assert!(
        input.is_object() && input.as_object().unwrap().is_empty(),
        "Input should be empty object for parameterless tool call"
    );

    assert_eq!(resp.stop_reason, Some("tool_use".to_string()));
}

// ===========================================================================
// Test 8: XML tool injection with existing system prompt
// ===========================================================================

#[tokio::test]
async fn xml_tool_injection_prepends_to_existing_system_prompt() {
    let (copilot_addr, captured, _h1) = spawn_capturing_mock().await;
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
        "system": "You are a coding assistant. Be concise.",
        "messages": [{"role": "user", "content": "Hello"}],
        "tools": [sample_tool()]
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

    let captured_body = captured.lock().await;
    let body = captured_body.as_ref().unwrap();
    let messages = body["messages"].as_array().unwrap();

    let system_content = messages[0]["content"].as_str().unwrap();

    // Tool XML should be prepended before the original system message
    assert!(
        system_content.contains("<tools>"),
        "Should contain XML tools"
    );
    assert!(
        system_content.contains("You are a coding assistant. Be concise."),
        "Original system prompt should be preserved"
    );

    // Tools should come before the original content
    let tools_pos = system_content.find("<tools>").unwrap();
    let original_pos = system_content.find("You are a coding assistant").unwrap();
    assert!(
        tools_pos < original_pos,
        "Tools should be prepended before original system content"
    );
}

// ===========================================================================
// Test 9: Tools are stripped from OpenAI request (not sent to Copilot API)
// ===========================================================================

#[tokio::test]
async fn xml_tools_stripped_from_copilot_request() {
    let (copilot_addr, captured, _h1) = spawn_capturing_mock().await;
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
        "tools": [sample_tool()],
        "tool_choice": "auto"
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

    let captured_body = captured.lock().await;
    let body = captured_body.as_ref().unwrap();

    // Copilot API should NOT receive tools or tool_choice fields
    assert!(
        body.get("tools").is_none() || body["tools"].is_null(),
        "tools should be stripped from Copilot request"
    );
    assert!(
        body.get("tool_choice").is_none() || body["tool_choice"].is_null(),
        "tool_choice should be stripped from Copilot request"
    );
}

// ===========================================================================
// Test 10: Malformed XML degrades gracefully — incomplete tags
// ===========================================================================

#[tokio::test]
async fn xml_malformed_incomplete_tags_degrade_gracefully() {
    let (copilot_addr, _h1) = spawn_mock_copilot_with_handler(
        |_headers, Json(body): Json<serde_json::Value>| async move {
            let model = body["model"].as_str().unwrap_or("gpt-4");
            // Incomplete XML — missing closing tags
            let content = r#"Let me try.

<function_calls>
<invoke name="get_weather">
<parameter name="location">London
</invoke>
</function_calls>

Done."#;

            Json(json!({
                "id": "chatcmpl-malformed2",
                "object": "chat.completion",
                "created": 1700000000,
                "model": model,
                "choices": [{
                    "index": 0,
                    "message": {"role": "assistant", "content": content},
                    "finish_reason": "stop"
                }],
                "usage": {"prompt_tokens": 10, "completion_tokens": 15, "total_tokens": 25}
            }))
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
        "messages": [{"role": "user", "content": "What's the weather?"}],
        "tools": [sample_tool()]
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

    // The response should parse without error even if tool parsing
    // produces partial or no results. "Graceful degradation" here means:
    // - No HTTP 500 error — the adapter returns a valid Anthropic response
    // - The response contains at least one content block (the surrounding
    //   prose text, possibly with the unparsed XML left inline)
    // - The malformed invoke may produce a partial `tool_use` block or be
    //   skipped entirely — both outcomes are acceptable
    assert_eq!(resp.response_type, "message");
    assert_eq!(resp.role, "assistant");

    let resp_json = serde_json::to_value(&resp).unwrap();
    let content = resp_json["content"].as_array().unwrap();
    assert!(
        !content.is_empty(),
        "Response should contain at least one content block (text or tool_use)"
    );
    // At minimum a text block should be present with the surrounding prose
    let has_text_block = content.iter().any(|b| b["type"] == "text");
    let has_tool_block = content.iter().any(|b| b["type"] == "tool_use");
    assert!(
        has_text_block || has_tool_block,
        "Response should contain a text or tool_use content block"
    );
}

// ===========================================================================
// Test 11: XML special characters in parameter values
// ===========================================================================

#[tokio::test]
async fn xml_special_characters_in_parameter_values() {
    let (copilot_addr, _h1) = spawn_mock_copilot_with_handler(
        |_headers, Json(body): Json<serde_json::Value>| async move {
            let model = body["model"].as_str().unwrap_or("gpt-4");
            // Parameter value contains &, <, > (XML-significant characters).
            // Models emit these as raw text, not XML-escaped.
            let content = r#"I'll run that command.

<function_calls>
<invoke name="bash">
<parameter name="command">if [ "$a" -lt 5 ] && echo "a < 5 & done > /dev/null"</parameter>
</invoke>
</function_calls>"#;

            Json(json!({
                "id": "chatcmpl-xmlchars",
                "object": "chat.completion",
                "created": 1700000000,
                "model": model,
                "choices": [{
                    "index": 0,
                    "message": {"role": "assistant", "content": content},
                    "finish_reason": "stop"
                }],
                "usage": {"prompt_tokens": 10, "completion_tokens": 20, "total_tokens": 30}
            }))
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
        "messages": [{"role": "user", "content": "Run the script"}],
        "tools": [{
            "name": "bash",
            "description": "Execute a bash command",
            "input_schema": {
                "type": "object",
                "properties": {
                    "command": {"type": "string", "description": "The command to run"}
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
    assert_eq!(
        tool_use_blocks.len(),
        1,
        "Should have exactly one tool_use block"
    );

    let tool_use = &tool_use_blocks[0];
    assert_eq!(tool_use["name"], "bash");

    // The parameter value should be round-tripped with special characters intact.
    // The parser uses a lazy regex that captures up to the first </parameter>,
    // so raw <, >, & in values should survive as-is.
    let command = tool_use["input"]["command"].as_str().unwrap();
    assert!(
        command.contains("&"),
        "Ampersand should be preserved in parameter value, got: {command}"
    );
    assert!(
        command.contains("<"),
        "Less-than should be preserved in parameter value, got: {command}"
    );
    assert!(
        command.contains(">"),
        "Greater-than should be preserved in parameter value, got: {command}"
    );

    assert_eq!(resp.stop_reason, Some("tool_use".to_string()));
}
