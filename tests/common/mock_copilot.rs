//! Reusable mock Copilot API server for integration tests.
//!
//! Provides configurable mock implementations of the Copilot chat completions
//! and models endpoints supporting both streaming (SSE) and non-streaming
//! responses.

use std::net::SocketAddr;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;

use axum::http::{HeaderMap, StatusCode};
use axum::routing::get;
use axum::{Json, Router};
use serde_json::json;
use tokio::net::TcpListener;
use tokio::task::JoinHandle;

// ---------------------------------------------------------------------------
// Mock Copilot models API (Epic 5 — E5-T1)
// ---------------------------------------------------------------------------

/// A running mock Copilot Models API server.
///
/// Provides configurable mock implementations of the Copilot models endpoint
/// (`GET /models`) for integration tests. Validates the same required headers
/// as the real Copilot API.
pub struct MockCopilotModels {
    pub addr: SocketAddr,
    #[allow(dead_code)] // Kept to prevent server from being dropped
    pub handle: JoinHandle<()>,
}

impl MockCopilotModels {
    /// Spawn a mock Copilot models API that returns a standard model list.
    /// Validates Authorization, Copilot-Integration-Id, and Editor-Version headers.
    pub async fn spawn() -> Self {
        let app = Router::new().route("/models", get(mock_models_handler));

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let handle = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });

        Self { addr, handle }
    }

    /// Spawn a mock Copilot models API with a request counter.
    /// Useful for verifying cache behaviour (e.g., no duplicate API calls).
    pub async fn spawn_with_counter() -> (Self, Arc<AtomicU32>) {
        let counter = Arc::new(AtomicU32::new(0));
        let counter_clone = counter.clone();

        let app = Router::new().route(
            "/models",
            get(move |headers: HeaderMap| {
                let c = counter_clone.clone();
                async move {
                    c.fetch_add(1, Ordering::SeqCst);
                    mock_models_handler(headers).await
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

    /// Spawn a mock Copilot models API that always returns HTTP 500.
    pub async fn spawn_failing() -> Self {
        let app = Router::new().route(
            "/models",
            get(|| async {
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

    /// URL for the models endpoint.
    pub fn models_url(&self) -> String {
        format!("http://{}/models", self.addr)
    }
}

/// Mock models handler: validates required Copilot headers, returns a model list.
async fn mock_models_handler(
    headers: HeaderMap,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
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
            Json(
                json!({"error": format!("expected Copilot-Integration-Id 'vscode-chat', got '{integration_id}'")}),
            ),
        ));
    }

    let editor_version = headers
        .get("Editor-Version")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    if editor_version != "vscode/1.85.0" {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(
                json!({"error": format!("expected Editor-Version 'vscode/1.85.0', got '{editor_version}'")}),
            ),
        ));
    }

    if headers.get("X-Request-Id").is_none() {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(json!({"error": "missing X-Request-Id header"})),
        ));
    }

    Ok(Json(json!({
        "object": "list",
        "data": [
            {
                "id": "gpt-4o",
                "object": "model",
                "created": 1715367049,
                "owned_by": "github-copilot"
            },
            {
                "id": "gpt-4",
                "object": "model",
                "created": 1686935002,
                "owned_by": "github-copilot"
            },
            {
                "id": "claude-sonnet-4",
                "object": "model",
                "created": 1715367049,
                "owned_by": "github-copilot"
            }
        ]
    })))
}

// ---------------------------------------------------------------------------
// Tool call mock helpers (Epic 7 — E7-T1)
// ---------------------------------------------------------------------------

/// Build a non-streaming response whose assistant content contains a single
/// tool call embedded in attribute-based XML.
///
/// The tool call uses the format the adapter's parser expects:
/// ```xml
/// <function_calls>
/// <invoke name="tool_name">
/// <parameter name="key">value</parameter>
/// </invoke>
/// </function_calls>
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

    // Build XML parameter elements from arguments
    let params_xml = if let Some(obj) = arguments.as_object() {
        obj.iter()
            .map(|(k, v)| {
                let val = match v {
                    serde_json::Value::String(s) => s.clone(),
                    other => other.to_string(),
                };
                format!("<parameter name=\"{k}\">{val}</parameter>")
            })
            .collect::<Vec<_>>()
            .join("\n")
    } else {
        String::new()
    };

    let content = format!(
        "{before}\n\n<function_calls>\n<invoke name=\"{tool_name}\">\n{params_xml}\n</invoke>\n</function_calls>\n\n{after}"
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
/// tool calls in XML format.
pub fn build_multi_tool_call_response(
    model: &str,
    tool_calls: &[(&str, serde_json::Value)],
) -> serde_json::Value {
    let mut content = String::from("I'll call several tools.\n\n<function_calls>\n");
    for (name, args) in tool_calls {
        content.push_str(&format!("<invoke name=\"{name}\">\n"));
        if let Some(obj) = args.as_object() {
            for (k, v) in obj {
                let val = match v {
                    serde_json::Value::String(s) => s.clone(),
                    other => other.to_string(),
                };
                content.push_str(&format!("<parameter name=\"{k}\">{val}</parameter>\n"));
            }
        }
        content.push_str("</invoke>\n");
    }
    content.push_str("</function_calls>\n\nDone.");

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
/// malformed XML (not a valid tool call).
pub fn build_malformed_tool_call_response(model: &str) -> serde_json::Value {
    // Broken XML: invoke with empty name (rejected by guard) and mangled tags
    let content = r#"Let me try to call a tool.

<function_calls>
<invoke name="">
<parameter name="location">London</parameter>
</invoke>
</function_calls>

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

/// Build SSE streaming chunks that include a tool call in XML format,
/// spread across multiple chunks.
pub fn build_streaming_tool_call_chunks(
    model: &str,
    tool_name: &str,
    arguments: serde_json::Value,
) -> String {
    // Build XML parameter elements
    let params_xml = if let Some(obj) = arguments.as_object() {
        obj.iter()
            .map(|(k, v)| {
                let val = match v {
                    serde_json::Value::String(s) => s.clone(),
                    other => other.to_string(),
                };
                format!("<parameter name=\"{k}\">{val}</parameter>")
            })
            .collect::<Vec<_>>()
            .join("\n")
    } else {
        String::new()
    };

    let xml_block = format!(
        "<function_calls>\n<invoke name=\"{tool_name}\">\n{params_xml}\n</invoke>\n</function_calls>"
    );

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
                "choices": [{"index": 0, "delta": {"content": xml_block}, "finish_reason": serde_json::Value::Null}]
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
// Native tool call mock helpers (Epic 0 — verification)
// ---------------------------------------------------------------------------

/// Build a non-streaming response with a native OpenAI-format tool call.
///
/// Unlike `build_tool_call_response` (which uses XML in text content), this
/// returns a response with structured `tool_calls` in the message — the format
/// expected when passing native tools to the Copilot API.
pub fn build_native_tool_call_response(
    model: &str,
    tool_name: &str,
    arguments: serde_json::Value,
    call_id: Option<&str>,
) -> serde_json::Value {
    let call_id = call_id.unwrap_or("call_native_mock_001");
    let args_str = serde_json::to_string(&arguments).unwrap_or_default();

    json!({
        "id": "chatcmpl-native-toolmock",
        "object": "chat.completion",
        "created": 1700000000,
        "model": model,
        "choices": [{
            "index": 0,
            "message": {
                "role": "assistant",
                "content": serde_json::Value::Null,
                "tool_calls": [{
                    "id": call_id,
                    "type": "function",
                    "function": {
                        "name": tool_name,
                        "arguments": args_str
                    }
                }]
            },
            "finish_reason": "tool_calls"
        }],
        "usage": {
            "prompt_tokens": 25,
            "completion_tokens": 15,
            "total_tokens": 40
        }
    })
}

/// Build a non-streaming response with multiple native tool calls.
pub fn build_native_multi_tool_call_response(
    model: &str,
    tool_calls: &[(&str, serde_json::Value)],
) -> serde_json::Value {
    let calls: Vec<serde_json::Value> = tool_calls
        .iter()
        .enumerate()
        .map(|(i, (name, args))| {
            let args_str = serde_json::to_string(args).unwrap_or_default();
            json!({
                "id": format!("call_native_mock_{:03}", i),
                "type": "function",
                "function": {
                    "name": name,
                    "arguments": args_str
                }
            })
        })
        .collect();

    json!({
        "id": "chatcmpl-native-multitool",
        "object": "chat.completion",
        "created": 1700000000,
        "model": model,
        "choices": [{
            "index": 0,
            "message": {
                "role": "assistant",
                "content": serde_json::Value::Null,
                "tool_calls": calls
            },
            "finish_reason": "tool_calls"
        }],
        "usage": {
            "prompt_tokens": 40,
            "completion_tokens": 30,
            "total_tokens": 70
        }
    })
}

/// Build SSE streaming chunks with native tool_calls deltas.
///
/// Simulates the OpenAI streaming format for tool calls:
/// 1. Role chunk (assistant)
/// 2. Tool call start (id, type, function name)
/// 3. Tool call arguments (partial JSON fragments)
/// 4. Finish reason chunk (tool_calls)
/// 5. [DONE]
pub fn build_native_streaming_tool_call_chunks(
    model: &str,
    tool_name: &str,
    arguments: serde_json::Value,
) -> String {
    let args_str = serde_json::to_string(&arguments).unwrap_or_default();

    // Split arguments into fragments to simulate streaming
    let (frag1, frag2) = if args_str.len() > 10 {
        let mid = args_str.len() / 2;
        (&args_str[..mid], &args_str[mid..])
    } else {
        (args_str.as_str(), "")
    };

    let mut chunks = vec![
        // Chunk 1: role
        format!(
            "data: {}\n\n",
            json!({
                "id": "chatcmpl-native-stream",
                "object": "chat.completion.chunk",
                "created": 1700000000,
                "model": model,
                "choices": [{"index": 0, "delta": {"role": "assistant"}, "finish_reason": serde_json::Value::Null}]
            })
        ),
        // Chunk 2: tool call start (id + name)
        format!(
            "data: {}\n\n",
            json!({
                "id": "chatcmpl-native-stream",
                "object": "chat.completion.chunk",
                "created": 1700000000,
                "model": model,
                "choices": [{"index": 0, "delta": {
                    "tool_calls": [{
                        "index": 0,
                        "id": "call_native_stream_001",
                        "type": "function",
                        "function": {"name": tool_name, "arguments": ""}
                    }]
                }, "finish_reason": serde_json::Value::Null}]
            })
        ),
        // Chunk 3: arguments fragment 1
        format!(
            "data: {}\n\n",
            json!({
                "id": "chatcmpl-native-stream",
                "object": "chat.completion.chunk",
                "created": 1700000000,
                "model": model,
                "choices": [{"index": 0, "delta": {
                    "tool_calls": [{
                        "index": 0,
                        "function": {"arguments": frag1}
                    }]
                }, "finish_reason": serde_json::Value::Null}]
            })
        ),
    ];

    // Chunk 4: arguments fragment 2 (if any)
    if !frag2.is_empty() {
        chunks.push(format!(
            "data: {}\n\n",
            json!({
                "id": "chatcmpl-native-stream",
                "object": "chat.completion.chunk",
                "created": 1700000000,
                "model": model,
                "choices": [{"index": 0, "delta": {
                    "tool_calls": [{
                        "index": 0,
                        "function": {"arguments": frag2}
                    }]
                }, "finish_reason": serde_json::Value::Null}]
            })
        ));
    }

    // Final chunk: finish_reason
    chunks.push(format!(
        "data: {}\n\n",
        json!({
            "id": "chatcmpl-native-stream",
            "object": "chat.completion.chunk",
            "created": 1700000000,
            "model": model,
            "choices": [{"index": 0, "delta": {}, "finish_reason": "tool_calls"}]
        })
    ));

    chunks.push("data: [DONE]\n\n".to_string());

    chunks.concat()
}

/// Build SSE streaming chunks with text content followed by a native tool call.
///
/// Simulates a response where the model first outputs some text, then makes
/// a tool call — testing the text-to-tool transition in streaming.
pub fn build_native_streaming_text_then_tool_chunks(
    model: &str,
    text: &str,
    tool_name: &str,
    arguments: serde_json::Value,
) -> String {
    let args_str = serde_json::to_string(&arguments).unwrap_or_default();

    let chunks = vec![
        // Role chunk
        format!(
            "data: {}\n\n",
            json!({
                "id": "chatcmpl-native-mixed",
                "object": "chat.completion.chunk",
                "created": 1700000000,
                "model": model,
                "choices": [{"index": 0, "delta": {"role": "assistant"}, "finish_reason": serde_json::Value::Null}]
            })
        ),
        // Text content chunk
        format!(
            "data: {}\n\n",
            json!({
                "id": "chatcmpl-native-mixed",
                "object": "chat.completion.chunk",
                "created": 1700000000,
                "model": model,
                "choices": [{"index": 0, "delta": {"content": text}, "finish_reason": serde_json::Value::Null}]
            })
        ),
        // Tool call start
        format!(
            "data: {}\n\n",
            json!({
                "id": "chatcmpl-native-mixed",
                "object": "chat.completion.chunk",
                "created": 1700000000,
                "model": model,
                "choices": [{"index": 0, "delta": {
                    "tool_calls": [{
                        "index": 0,
                        "id": "call_native_mixed_001",
                        "type": "function",
                        "function": {"name": tool_name, "arguments": ""}
                    }]
                }, "finish_reason": serde_json::Value::Null}]
            })
        ),
        // Tool call arguments
        format!(
            "data: {}\n\n",
            json!({
                "id": "chatcmpl-native-mixed",
                "object": "chat.completion.chunk",
                "created": 1700000000,
                "model": model,
                "choices": [{"index": 0, "delta": {
                    "tool_calls": [{
                        "index": 0,
                        "function": {"arguments": args_str}
                    }]
                }, "finish_reason": serde_json::Value::Null}]
            })
        ),
        // Finish
        format!(
            "data: {}\n\n",
            json!({
                "id": "chatcmpl-native-mixed",
                "object": "chat.completion.chunk",
                "created": 1700000000,
                "model": model,
                "choices": [{"index": 0, "delta": {}, "finish_reason": "tool_calls"}]
            })
        ),
        "data: [DONE]\n\n".to_string(),
    ];

    chunks.concat()
}
