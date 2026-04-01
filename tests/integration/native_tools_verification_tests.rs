//! Integration tests for native OpenAI tool call support verification.
//!
//! Epic 0: These tests spin up a mock Copilot server that returns native
//! tool_calls (not XML), and verify that the CopilotClient correctly handles
//! the responses. This validates that the adapter's type system and client
//! infrastructure can handle native tool responses from the Copilot API.

use std::sync::Arc;

use axum::body::Body;
use axum::http::HeaderMap;
use axum::response::IntoResponse;
use axum::routing::post;
use axum::{Json, Router};
use futures::StreamExt;
use serde_json::json;
use tokio::net::TcpListener;

use copilot_adapter::copilot::client::CopilotClient;
use copilot_adapter::copilot::types::ChatCompletionRequest;

use crate::common::mock_copilot::{
    build_native_multi_tool_call_response, build_native_streaming_text_then_tool_chunks,
    build_native_streaming_tool_call_chunks, build_native_tool_call_response, parse_sse_body,
};

// ---------------------------------------------------------------------------
// Helper: spawn a mock Copilot server with a custom POST handler
// ---------------------------------------------------------------------------

async fn spawn_mock_with_handler<F, Fut>(handler: F) -> (String, tokio::task::JoinHandle<()>)
where
    F: Fn(HeaderMap, Json<serde_json::Value>) -> Fut + Clone + Send + Sync + 'static,
    Fut: std::future::Future<Output = axum::response::Response> + Send,
{
    let app = Router::new().route(
        "/chat/completions",
        post(move |headers: HeaderMap, body: Json<serde_json::Value>| {
            let h = handler.clone();
            async move { h(headers, body).await }
        }),
    );

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let handle = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });

    (format!("http://{addr}/chat/completions"), handle)
}

// ===========================================================================
// E0-T1: Non-streaming native tool call via CopilotClient
// ===========================================================================

#[tokio::test]
async fn copilot_client_receives_native_tool_call_non_streaming() {
    let (url, _handle) =
        spawn_mock_with_handler(|_headers, Json(body): Json<serde_json::Value>| async move {
            let model = body["model"].as_str().unwrap_or("gpt-4o");

            // Verify that tools were forwarded in the request
            assert!(
                body.get("tools").is_some(),
                "Request should include tools field"
            );
            let tools = body["tools"].as_array().unwrap();
            assert!(!tools.is_empty(), "Tools array should not be empty");

            Json(build_native_tool_call_response(
                model,
                "get_weather",
                json!({"location": "London"}),
                None,
            ))
            .into_response()
        })
        .await;

    let client = CopilotClient::with_api_url(reqwest::Client::new(), url);

    let request = ChatCompletionRequest {
        model: "gpt-4o".to_string(),
        messages: vec![copilot_adapter::copilot::types::Message {
            role: "user".to_string(),
            content: copilot_adapter::copilot::types::MessageContent::Text(
                "Get the weather in London".to_string(),
            ),
            name: None,
            tool_calls: None,
            tool_call_id: None,
        }],
        stream: Some(false),
        temperature: None,
        max_tokens: None,
        top_p: None,
        n: None,
        stop: None,
        presence_penalty: None,
        frequency_penalty: None,
        tools: Some(vec![copilot_adapter::copilot::types::OpenAITool {
            tool_type: "function".to_string(),
            function: copilot_adapter::copilot::types::OpenAIToolFunction {
                name: "get_weather".to_string(),
                description: Some("Get weather for a location".to_string()),
                parameters: Some(json!({
                    "type": "object",
                    "properties": {
                        "location": {"type": "string"}
                    },
                    "required": ["location"]
                })),
            },
        }]),
        tool_choice: Some(json!("auto")),
    };

    let response = client
        .send_chat_completion("test_token", &request)
        .await
        .unwrap();

    // Verify the response contains native tool calls
    let choice = &response.choices[0];
    assert_eq!(choice.finish_reason, Some("tool_calls".to_string()));

    let tool_calls = choice.message.tool_calls.as_ref().unwrap();
    assert_eq!(tool_calls.len(), 1);

    let tc = &tool_calls[0];
    assert_eq!(tc.function.name, Some("get_weather".to_string()));
    assert!(tc.id.is_some());
    assert_eq!(tc.call_type, Some("function".to_string()));

    // Verify arguments preserve types
    let args: serde_json::Value =
        serde_json::from_str(tc.function.arguments.as_ref().unwrap()).unwrap();
    assert_eq!(args["location"], "London");
}

// ===========================================================================
// E0-T1 (continued): Multiple native tool calls
// ===========================================================================

#[tokio::test]
async fn copilot_client_receives_multiple_native_tool_calls() {
    let (url, _handle) =
        spawn_mock_with_handler(|_headers, Json(body): Json<serde_json::Value>| async move {
            // Verify tools were forwarded in the request
            assert!(
                body.get("tools").is_some(),
                "Request should include tools field"
            );
            let tools = body["tools"].as_array().unwrap();
            assert_eq!(
                tools.len(),
                2,
                "Expected both tool definitions to be forwarded"
            );

            let model = body["model"].as_str().unwrap_or("gpt-4o");
            Json(build_native_multi_tool_call_response(
                model,
                &[
                    ("get_weather", json!({"location": "London"})),
                    ("read_file", json!({"path": "/tmp/test.txt"})),
                ],
            ))
            .into_response()
        })
        .await;

    let client = CopilotClient::with_api_url(reqwest::Client::new(), url);

    let request = ChatCompletionRequest {
        model: "gpt-4o".to_string(),
        messages: vec![copilot_adapter::copilot::types::Message {
            role: "user".to_string(),
            content: copilot_adapter::copilot::types::MessageContent::Text(
                "Get weather and read file".to_string(),
            ),
            name: None,
            tool_calls: None,
            tool_call_id: None,
        }],
        stream: None,
        temperature: None,
        max_tokens: None,
        top_p: None,
        n: None,
        stop: None,
        presence_penalty: None,
        frequency_penalty: None,
        tools: Some(vec![
            copilot_adapter::copilot::types::OpenAITool {
                tool_type: "function".to_string(),
                function: copilot_adapter::copilot::types::OpenAIToolFunction {
                    name: "get_weather".to_string(),
                    description: Some("Get weather for a location".to_string()),
                    parameters: Some(json!({
                        "type": "object",
                        "properties": {
                            "location": {"type": "string"}
                        },
                        "required": ["location"]
                    })),
                },
            },
            copilot_adapter::copilot::types::OpenAITool {
                tool_type: "function".to_string(),
                function: copilot_adapter::copilot::types::OpenAIToolFunction {
                    name: "read_file".to_string(),
                    description: Some("Read a file from disk".to_string()),
                    parameters: Some(json!({
                        "type": "object",
                        "properties": {
                            "path": {"type": "string"}
                        },
                        "required": ["path"]
                    })),
                },
            },
        ]),
        tool_choice: Some(json!("auto")),
    };

    let response = client
        .send_chat_completion("test_token", &request)
        .await
        .unwrap();

    let choice = &response.choices[0];
    assert_eq!(choice.finish_reason, Some("tool_calls".to_string()));

    let tool_calls = choice.message.tool_calls.as_ref().unwrap();
    assert_eq!(tool_calls.len(), 2);

    // First tool call: get_weather
    assert_eq!(tool_calls[0].function.name, Some("get_weather".to_string()));
    assert!(
        tool_calls[0].id.is_some(),
        "First tool call should have an ID"
    );
    assert_eq!(tool_calls[0].call_type, Some("function".to_string()));
    let args0: serde_json::Value =
        serde_json::from_str(tool_calls[0].function.arguments.as_ref().unwrap()).unwrap();
    assert_eq!(args0["location"], "London");

    // Second tool call: read_file
    assert_eq!(tool_calls[1].function.name, Some("read_file".to_string()));
    assert!(
        tool_calls[1].id.is_some(),
        "Second tool call should have an ID"
    );
    assert_eq!(tool_calls[1].call_type, Some("function".to_string()));
    let args1: serde_json::Value =
        serde_json::from_str(tool_calls[1].function.arguments.as_ref().unwrap()).unwrap();
    assert_eq!(args1["path"], "/tmp/test.txt");
}

// ===========================================================================
// E0-T3: Streaming native tool calls via CopilotClient
// ===========================================================================

#[tokio::test]
async fn copilot_client_streams_native_tool_call() {
    let (url, _handle) =
        spawn_mock_with_handler(|_headers, Json(body): Json<serde_json::Value>| async move {
            let model = body["model"].as_str().unwrap_or("gpt-4o");
            let sse_body = build_native_streaming_tool_call_chunks(
                model,
                "get_weather",
                json!({"location": "London"}),
            );

            axum::http::Response::builder()
                .status(200)
                .header("Content-Type", "text/event-stream")
                .body(Body::from(sse_body))
                .unwrap()
                .into_response()
        })
        .await;

    let client = CopilotClient::with_api_url(reqwest::Client::new(), url);

    let request = ChatCompletionRequest {
        model: "gpt-4o".to_string(),
        messages: vec![copilot_adapter::copilot::types::Message {
            role: "user".to_string(),
            content: copilot_adapter::copilot::types::MessageContent::Text(
                "Get the weather".to_string(),
            ),
            name: None,
            tool_calls: None,
            tool_call_id: None,
        }],
        stream: Some(true),
        temperature: None,
        max_tokens: None,
        top_p: None,
        n: None,
        stop: None,
        presence_penalty: None,
        frequency_penalty: None,
        tools: None,
        tool_choice: None,
    };

    let stream = client
        .stream_chat_completion("test_token", &request)
        .await
        .unwrap();

    let chunks: Vec<_> = stream
        .collect::<Vec<_>>()
        .await
        .into_iter()
        .map(|r| r.unwrap())
        .collect();

    // Should have multiple chunks
    assert!(
        chunks.len() >= 3,
        "Expected at least 3 chunks, got {}",
        chunks.len()
    );

    // Reconstruct the tool call from streaming deltas
    let mut call_id = String::new();
    let mut call_name = String::new();
    let mut call_args = String::new();
    let mut saw_tool_calls_finish = false;

    for chunk in &chunks {
        let choice = &chunk.choices[0];

        if let Some(tool_calls) = &choice.delta.tool_calls {
            for tc in tool_calls {
                if let Some(id) = &tc.id {
                    call_id = id.clone();
                }
                if let Some(func) = &tc.function {
                    if let Some(name) = &func.name {
                        call_name = name.clone();
                    }
                    if let Some(args) = &func.arguments {
                        call_args.push_str(args);
                    }
                }
            }
        }

        if choice.finish_reason.as_deref() == Some("tool_calls") {
            saw_tool_calls_finish = true;
        }
    }

    assert_eq!(call_name, "get_weather");
    assert!(!call_id.is_empty(), "Tool call ID should be present");
    assert!(
        saw_tool_calls_finish,
        "Should see finish_reason 'tool_calls'"
    );

    // Verify the accumulated arguments are valid JSON
    let parsed_args: serde_json::Value = serde_json::from_str(&call_args).unwrap();
    assert_eq!(parsed_args["location"], "London");
}

// ===========================================================================
// E0-T3 (continued): Streaming with text then tool call
// ===========================================================================

#[tokio::test]
async fn copilot_client_streams_text_then_native_tool_call() {
    let (url, _handle) =
        spawn_mock_with_handler(|_headers, Json(body): Json<serde_json::Value>| async move {
            let model = body["model"].as_str().unwrap_or("gpt-4o");
            let sse_body = build_native_streaming_text_then_tool_chunks(
                model,
                "I'll check the weather for you.",
                "get_weather",
                json!({"location": "London"}),
            );

            axum::http::Response::builder()
                .status(200)
                .header("Content-Type", "text/event-stream")
                .body(Body::from(sse_body))
                .unwrap()
                .into_response()
        })
        .await;

    let client = CopilotClient::with_api_url(reqwest::Client::new(), url);

    let request = ChatCompletionRequest {
        model: "gpt-4o".to_string(),
        messages: vec![copilot_adapter::copilot::types::Message {
            role: "user".to_string(),
            content: copilot_adapter::copilot::types::MessageContent::Text(
                "Get the weather".to_string(),
            ),
            name: None,
            tool_calls: None,
            tool_call_id: None,
        }],
        stream: Some(true),
        temperature: None,
        max_tokens: None,
        top_p: None,
        n: None,
        stop: None,
        presence_penalty: None,
        frequency_penalty: None,
        tools: None,
        tool_choice: None,
    };

    let stream = client
        .stream_chat_completion("test_token", &request)
        .await
        .unwrap();

    let chunks: Vec<_> = stream
        .collect::<Vec<_>>()
        .await
        .into_iter()
        .map(|r| r.unwrap())
        .collect();

    // Should have text chunks AND tool call chunks
    let mut has_text = false;
    let mut has_tool_call = false;
    let mut text_content = String::new();

    for chunk in &chunks {
        let choice = &chunk.choices[0];
        if let Some(text) = &choice.delta.content {
            has_text = true;
            text_content.push_str(text);
        }
        if let Some(tool_calls) = &choice.delta.tool_calls {
            if !tool_calls.is_empty() {
                has_tool_call = true;
            }
        }
    }

    assert!(has_text, "Should have text content in stream");
    assert!(has_tool_call, "Should have tool_calls in stream");
    assert!(
        text_content.contains("check the weather"),
        "Text should contain the expected content"
    );
}

// ===========================================================================
// E0-T4: Verify tools field is forwarded in the request body
// ===========================================================================

#[tokio::test]
async fn copilot_client_forwards_tools_in_request_body() {
    let received_body = Arc::new(tokio::sync::Mutex::new(None::<serde_json::Value>));
    let received_body_clone = received_body.clone();

    let (url, _handle) =
        spawn_mock_with_handler(move |_headers, Json(body): Json<serde_json::Value>| {
            let rb = received_body_clone.clone();
            async move {
                *rb.lock().await = Some(body.clone());
                let model = body["model"].as_str().unwrap_or("gpt-4o");
                Json(build_native_tool_call_response(
                    model,
                    "get_weather",
                    json!({"location": "London"}),
                    None,
                ))
                .into_response()
            }
        })
        .await;

    let client = CopilotClient::with_api_url(reqwest::Client::new(), url);

    let request = ChatCompletionRequest {
        model: "gpt-4o".to_string(),
        messages: vec![copilot_adapter::copilot::types::Message {
            role: "user".to_string(),
            content: copilot_adapter::copilot::types::MessageContent::Text(
                "Get the weather".to_string(),
            ),
            name: None,
            tool_calls: None,
            tool_call_id: None,
        }],
        stream: Some(false),
        temperature: None,
        max_tokens: None,
        top_p: None,
        n: None,
        stop: None,
        presence_penalty: None,
        frequency_penalty: None,
        tools: Some(vec![copilot_adapter::copilot::types::OpenAITool {
            tool_type: "function".to_string(),
            function: copilot_adapter::copilot::types::OpenAIToolFunction {
                name: "get_weather".to_string(),
                description: Some("Get weather".to_string()),
                parameters: Some(json!({
                    "type": "object",
                    "properties": {
                        "location": {"type": "string"}
                    }
                })),
            },
        }]),
        tool_choice: Some(json!("auto")),
    };

    let _response = client
        .send_chat_completion("test_token", &request)
        .await
        .unwrap();

    // Verify the request body that was sent to the mock server
    let body = received_body.lock().await;
    let body = body.as_ref().expect("Should have received request body");

    // Tools should be present in the forwarded request
    let tools = body["tools"].as_array().expect("tools should be an array");
    assert_eq!(tools.len(), 1);
    assert_eq!(tools[0]["type"], "function");
    assert_eq!(tools[0]["function"]["name"], "get_weather");

    // tool_choice should be present
    assert_eq!(body["tool_choice"], "auto");
}
