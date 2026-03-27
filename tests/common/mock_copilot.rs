//! Reusable mock Copilot API server for integration tests.
//!
//! Provides configurable mock implementations of the Copilot chat completions
//! endpoint supporting both streaming (SSE) and non-streaming responses.

use std::net::SocketAddr;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;

use axum::body::Body;
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::post;
use axum::{Json, Router};
use serde_json::json;
use tokio::net::TcpListener;
use tokio::task::JoinHandle;

use copilot_adapter::copilot::types::ChatCompletionChunk;

/// A running mock Copilot API server.
pub struct MockCopilot {
    pub addr: SocketAddr,
    pub handle: JoinHandle<()>,
}

impl MockCopilot {
    /// Spawn a mock Copilot API that handles both streaming and non-streaming
    /// chat completion requests. Validates required headers.
    pub async fn spawn() -> Self {
        let app = Router::new().route("/chat/completions", post(mock_chat_completions));

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let handle = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });

        Self { addr, handle }
    }

    /// Spawn a mock Copilot API with a request counter.
    pub async fn spawn_with_counter() -> (Self, Arc<AtomicU32>) {
        let counter = Arc::new(AtomicU32::new(0));
        let counter_clone = counter.clone();

        let app = Router::new().route(
            "/chat/completions",
            post(move |headers: HeaderMap, body: Json<serde_json::Value>| {
                let c = counter_clone.clone();
                async move {
                    c.fetch_add(1, Ordering::SeqCst);
                    mock_chat_completions(headers, body).await
                }
            }),
        );

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let handle = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });

        (Self { addr, handle }, counter)
    }

    /// Spawn a mock Copilot API that always returns HTTP 500.
    pub async fn spawn_failing() -> Self {
        let app = Router::new().route(
            "/chat/completions",
            post(|| async {
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(json!({"error": "internal server error"})),
                )
            }),
        );

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let handle = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });

        Self { addr, handle }
    }

    /// Spawn a mock Copilot API that returns HTTP 429 with a Retry-After header.
    pub async fn spawn_rate_limited(retry_after: u64) -> Self {
        let app = Router::new().route(
            "/chat/completions",
            post(move || async move {
                let mut response = (
                    StatusCode::TOO_MANY_REQUESTS,
                    Json(json!({"error": "rate limited"})),
                )
                    .into_response();
                response.headers_mut().insert(
                    "Retry-After",
                    axum::http::HeaderValue::from_str(&retry_after.to_string()).unwrap(),
                );
                response
            }),
        );

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let handle = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });

        Self { addr, handle }
    }

    /// Spawn a mock Copilot API that adds a configurable delay before responding.
    pub async fn spawn_slow(delay_ms: u64) -> Self {
        let app = Router::new().route(
            "/chat/completions",
            post(move |headers: HeaderMap, body: Json<serde_json::Value>| async move {
                tokio::time::sleep(std::time::Duration::from_millis(delay_ms)).await;
                mock_chat_completions(headers, body).await
            }),
        );

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let handle = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });

        Self { addr, handle }
    }

    /// URL for the chat completions endpoint.
    pub fn completions_url(&self) -> String {
        format!("http://{}/chat/completions", self.addr)
    }
}

/// Mock chat completions handler: validates headers, branches on stream field.
async fn mock_chat_completions(
    headers: HeaderMap,
    Json(body): Json<serde_json::Value>,
) -> Result<Response, (StatusCode, Json<serde_json::Value>)> {
    // Validate Authorization header
    let auth = headers
        .get("Authorization")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    if !auth.starts_with("Bearer ") {
        return Err((
            StatusCode::UNAUTHORIZED,
            Json(json!({"error": "missing Bearer token"})),
        ));
    }

    // Validate required Copilot headers
    let integration_id = headers
        .get("Copilot-Integration-Id")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    if integration_id != "vscode-chat" {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(json!({"error": format!("expected Copilot-Integration-Id 'vscode-chat', got '{integration_id}'")})),
        ));
    }

    let editor_version = headers
        .get("Editor-Version")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    if editor_version != "vscode/1.85.0" {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(json!({"error": format!("expected Editor-Version 'vscode/1.85.0', got '{editor_version}'")})),
        ));
    }

    if headers.get("X-Request-Id").is_none() {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(json!({"error": "missing X-Request-Id header"})),
        ));
    }

    let model = body["model"].as_str().unwrap_or("gpt-4");
    let stream = body["stream"].as_bool().unwrap_or(false);

    if stream {
        return Ok(build_streaming_response(model));
    }

    Ok(build_non_streaming_response(model))
}

/// Build a non-streaming JSON chat completion response.
fn build_non_streaming_response(model: &str) -> Response {
    Json(json!({
        "id": "chatcmpl-mock123",
        "object": "chat.completion",
        "created": 1700000000,
        "model": model,
        "choices": [{
            "index": 0,
            "message": {
                "role": "assistant",
                "content": "Hello from mock Copilot!"
            },
            "finish_reason": "stop"
        }],
        "usage": {
            "prompt_tokens": 10,
            "completion_tokens": 5,
            "total_tokens": 15
        }
    }))
    .into_response()
}

/// Build a streaming SSE chat completion response with 3 chunks + [DONE].
fn build_streaming_response(model: &str) -> Response {
    let chunks = vec![
        format!(
            "data: {}\n\n",
            json!({
                "id": "chatcmpl-mock123",
                "object": "chat.completion.chunk",
                "created": 1700000000,
                "model": model,
                "choices": [{"index": 0, "delta": {"role": "assistant"}, "finish_reason": null}]
            })
        ),
        format!(
            "data: {}\n\n",
            json!({
                "id": "chatcmpl-mock123",
                "object": "chat.completion.chunk",
                "created": 1700000000,
                "model": model,
                "choices": [{"index": 0, "delta": {"content": "Hello from mock Copilot!"}, "finish_reason": null}]
            })
        ),
        format!(
            "data: {}\n\n",
            json!({
                "id": "chatcmpl-mock123",
                "object": "chat.completion.chunk",
                "created": 1700000000,
                "model": model,
                "choices": [{"index": 0, "delta": {}, "finish_reason": "stop"}]
            })
        ),
        "data: [DONE]\n\n".to_string(),
    ];

    axum::http::Response::builder()
        .status(200)
        .header("Content-Type", "text/event-stream")
        .body(Body::from(chunks.concat()))
        .unwrap()
        .into_response()
}

// ---------------------------------------------------------------------------
// Tool call mock helpers (Epic 7 — E7-T1)
// ---------------------------------------------------------------------------

/// Build a non-streaming response whose assistant content contains a single
/// tool call embedded in a fenced ```json code block.
///
/// The tool call uses the format the adapter's parser expects:
/// ```json
/// {"function_call": {"name": "<tool_name>", "arguments": <args>}}
/// ```
pub fn build_tool_call_response(
    model: &str,
    tool_name: &str,
    arguments: serde_json::Value,
    surrounding_text: Option<(&str, &str)>,
) -> serde_json::Value {
    let (before, after) = surrounding_text.unwrap_or((
        "I'll call the tool for you.",
        "Let me know if you need anything else.",
    ));
    let tool_json = serde_json::json!({
        "function_call": {
            "name": tool_name,
            "arguments": arguments
        }
    });

    let content = format!(
        "{before}\n\n```json\n{}\n```\n\n{after}",
        serde_json::to_string(&tool_json).unwrap()
    );

    serde_json::json!({
        "id": "chatcmpl-toolmock",
        "object": "chat.completion",
        "created": 1700000000,
        "model": model,
        "choices": [{
            "index": 0,
            "message": {
                "role": "assistant",
                "content": content
            },
            "finish_reason": "stop"
        }],
        "usage": {
            "prompt_tokens": 20,
            "completion_tokens": 30,
            "total_tokens": 50
        }
    })
}

/// Build a non-streaming response whose assistant content contains multiple
/// tool calls in separate fenced ```json code blocks.
pub fn build_multi_tool_call_response(
    model: &str,
    tool_calls: &[(&str, serde_json::Value)],
) -> serde_json::Value {
    let mut content = String::from("I'll call several tools.\n");
    for (name, args) in tool_calls {
        let tool_json = serde_json::json!({
            "function_call": {
                "name": name,
                "arguments": args
            }
        });
        content.push_str(&format!(
            "\n```json\n{}\n```\n",
            serde_json::to_string(&tool_json).unwrap()
        ));
    }
    content.push_str("\nDone.");

    serde_json::json!({
        "id": "chatcmpl-multitool",
        "object": "chat.completion",
        "created": 1700000000,
        "model": model,
        "choices": [{
            "index": 0,
            "message": {
                "role": "assistant",
                "content": content
            },
            "finish_reason": "stop"
        }],
        "usage": {
            "prompt_tokens": 30,
            "completion_tokens": 60,
            "total_tokens": 90
        }
    })
}

/// Build a non-streaming response whose assistant content contains
/// malformed JSON inside a fenced code block (not a valid tool call).
pub fn build_malformed_tool_call_response(model: &str) -> serde_json::Value {
    let content = r#"Let me try to call a tool.

```json
{"function_call": {"name": "get_weather", "arguments": {invalid json here}}}
```

I hope that worked."#;

    serde_json::json!({
        "id": "chatcmpl-malformed",
        "object": "chat.completion",
        "created": 1700000000,
        "model": model,
        "choices": [{
            "index": 0,
            "message": {
                "role": "assistant",
                "content": content
            },
            "finish_reason": "stop"
        }],
        "usage": {
            "prompt_tokens": 15,
            "completion_tokens": 20,
            "total_tokens": 35
        }
    })
}

/// Build a plain non-streaming response with no tool calls.
pub fn build_plain_response(model: &str, content: &str) -> serde_json::Value {
    serde_json::json!({
        "id": "chatcmpl-plain",
        "object": "chat.completion",
        "created": 1700000000,
        "model": model,
        "choices": [{
            "index": 0,
            "message": {
                "role": "assistant",
                "content": content
            },
            "finish_reason": "stop"
        }],
        "usage": {
            "prompt_tokens": 10,
            "completion_tokens": 5,
            "total_tokens": 15
        }
    })
}

/// Build SSE streaming chunks that include a tool call in a fenced JSON
/// block, spread across multiple chunks.
pub fn build_streaming_tool_call_chunks(
    model: &str,
    tool_name: &str,
    arguments: serde_json::Value,
) -> String {
    let tool_json = serde_json::json!({
        "function_call": {
            "name": tool_name,
            "arguments": arguments
        }
    });

    let chunks = vec![
        format!(
            "data: {}\n\n",
            serde_json::json!({
                "id": "chatcmpl-stream-toolmock",
                "object": "chat.completion.chunk",
                "created": 1700000000,
                "model": model,
                "choices": [{"index": 0, "delta": {"role": "assistant"}, "finish_reason": serde_json::Value::Null}]
            })
        ),
        format!(
            "data: {}\n\n",
            serde_json::json!({
                "id": "chatcmpl-stream-toolmock",
                "object": "chat.completion.chunk",
                "created": 1700000000,
                "model": model,
                "choices": [{"index": 0, "delta": {"content": "I'll call the tool.\n\n"}, "finish_reason": serde_json::Value::Null}]
            })
        ),
        format!(
            "data: {}\n\n",
            serde_json::json!({
                "id": "chatcmpl-stream-toolmock",
                "object": "chat.completion.chunk",
                "created": 1700000000,
                "model": model,
                "choices": [{"index": 0, "delta": {"content": format!("```json\n{}\n```", serde_json::to_string(&tool_json).unwrap())}, "finish_reason": serde_json::Value::Null}]
            })
        ),
        format!(
            "data: {}\n\n",
            serde_json::json!({
                "id": "chatcmpl-stream-toolmock",
                "object": "chat.completion.chunk",
                "created": 1700000000,
                "model": model,
                "choices": [{"index": 0, "delta": {"content": "\n\nDone."}, "finish_reason": "stop"}]
            })
        ),
        "data: [DONE]\n\n".to_string(),
    ];

    chunks.concat()
}

// ---------------------------------------------------------------------------
// SSE parsing utility
// ---------------------------------------------------------------------------

/// Parse raw SSE body text into a list of chunks and a `saw_done` flag.
/// Shared utility for test assertions.
pub fn parse_sse_body(body_text: &str) -> (Vec<ChatCompletionChunk>, bool) {
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
