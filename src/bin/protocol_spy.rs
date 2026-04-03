//! Protocol Spy — Transparent pass-through proxy for inspecting the Anthropic Messages API.
//!
//! Sits between Claude Code and any upstream (copilot-adapter, Anthropic API, etc.),
//! forwarding all traffic verbatim while logging requests, responses, and SSE events
//! to both the console (with colors) and JSON files.

use axum::{
    body::Body,
    extract::State,
    http::{HeaderMap, Response, StatusCode},
    Router,
};
use bytes::{Bytes, BytesMut};
use chrono::Utc;
use clap::Parser;
use colored::*;
use futures::StreamExt;
use serde_json::Value;
use std::{
    path::PathBuf,
    sync::{Arc, Mutex},
    time::Instant,
};
use tokio::net::TcpListener;
use tracing_subscriber::EnvFilter;

// ─── CLI ─────────────────────────────────────────────────────────────────────

#[derive(Parser, Debug)]
#[command(
    name = "protocol-spy",
    about = "Transparent protocol inspector for the Anthropic Messages API",
    version
)]
struct Cli {
    /// Upstream server URL to forward requests to
    #[arg(short, long, default_value = "http://localhost:6767")]
    upstream: String,

    /// Port to listen on
    #[arg(short, long, default_value_t = 6780)]
    port: u16,

    /// Host to bind to
    #[arg(long, default_value = "127.0.0.1")]
    host: String,

    /// Directory for JSON log files
    #[arg(long, default_value = "protocol-spy-logs")]
    log_dir: String,

    /// Suppress colored console output
    #[arg(long)]
    no_console: bool,

    /// Suppress JSON file logging
    #[arg(long)]
    no_file: bool,

    /// Log level for tracing infrastructure (not the spy output)
    #[arg(long, default_value = "info")]
    log_level: String,
}

// ─── State ───────────────────────────────────────────────────────────────────

struct SpyState {
    upstream: String,
    client: reqwest::Client,
    log_dir: PathBuf,
    console_enabled: bool,
    file_enabled: bool,
}

// ─── Main ────────────────────────────────────────────────────────────────────

#[tokio::main]
async fn main() {
    let cli = Cli::parse();

    // Initialize tracing (for the spy's own operational logs, not the protocol logs)
    let filter = EnvFilter::try_new(&cli.log_level).unwrap_or_else(|_| EnvFilter::new("info"));
    tracing_subscriber::fmt().with_env_filter(filter).init();

    let state = Arc::new(SpyState {
        upstream: cli.upstream.trim_end_matches('/').to_string(),
        client: reqwest::Client::builder()
            .no_proxy()
            .build()
            .expect("Failed to create HTTP client"),
        log_dir: PathBuf::from(&cli.log_dir),
        console_enabled: !cli.no_console,
        file_enabled: !cli.no_file,
    });

    // Create log directory if file logging is enabled
    if state.file_enabled {
        if let Err(e) = std::fs::create_dir_all(&state.log_dir) {
            eprintln!("Warning: could not create log directory {:?}: {}", state.log_dir, e);
        }
    }

    let app = Router::new()
        .fallback(proxy_handler)
        .with_state(state.clone());

    let addr = format!("{}:{}", cli.host, cli.port);
    print_banner(&addr, &state.upstream, &cli.log_dir, state.console_enabled, state.file_enabled);

    let listener = TcpListener::bind(&addr)
        .await
        .unwrap_or_else(|e| panic!("Failed to bind to {}: {}", addr, e));

    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await
        .unwrap();
}

async fn shutdown_signal() {
    tokio::signal::ctrl_c()
        .await
        .expect("Failed to install Ctrl+C handler");
    eprintln!("\nShutting down...");
}

// ─── Startup Banner ──────────────────────────────────────────────────────────

fn print_banner(addr: &str, upstream: &str, log_dir: &str, console: bool, file: bool) {
    let outputs = match (console, file) {
        (true, true) => "console + files",
        (true, false) => "console only",
        (false, true) => "files only",
        (false, false) => "NONE (all logging disabled!)",
    };

    eprintln!();
    eprintln!("{}", "  ┌──────────────────────────────────────────────────┐".cyan());
    eprintln!("{}", format!("  │  {} {}          │",
        "Protocol Spy".cyan().bold(),
        env!("CARGO_PKG_VERSION").dimmed()
    ));
    eprintln!("  │  Listening: {:<37}│", format!("http://{}", addr).green());
    eprintln!("  │  Upstream:  {:<37}│", upstream.yellow());
    eprintln!("  │  Log dir:   {:<37}│", log_dir.dimmed());
    eprintln!("  │  Output:    {:<37}│", outputs);
    eprintln!("  │{}│", " ".repeat(50));
    eprintln!("  │  {}  │", "Configure Claude Code:".white().bold());
    eprintln!("  │  ANTHROPIC_BASE_URL=http://{:<21}│", addr);
    eprintln!("{}", "  └──────────────────────────────────────────────────┘".cyan());
    eprintln!();
}

// ─── Proxy Handler ───────────────────────────────────────────────────────────

async fn proxy_handler(
    State(state): State<Arc<SpyState>>,
    req: axum::extract::Request,
) -> Result<Response<Body>, StatusCode> {
    let start = Instant::now();
    let request_id = uuid::Uuid::new_v4().to_string();
    let short_id = &request_id[..8];
    let timestamp = Utc::now();
    let ts_str = timestamp.format("%Y-%m-%dT%H:%M:%S%.3fZ").to_string();

    // ── 1. Capture incoming request ──────────────────────────────────────────
    let method = req.method().clone();
    let uri = req.uri().clone();
    let path_and_query = uri
        .path_and_query()
        .map(|pq| pq.as_str().to_string())
        .unwrap_or_else(|| "/".to_string());
    let req_headers = req.headers().clone();

    let body_bytes = axum::body::to_bytes(req.into_body(), 50 * 1024 * 1024)
        .await
        .map_err(|e| {
            tracing::error!("Failed to read request body: {}", e);
            StatusCode::BAD_REQUEST
        })?;

    // ── 2. Log request ──────────────────────────────────────────────────────
    if state.console_enabled {
        log_request_console(short_id, &ts_str, &method.to_string(), &path_and_query, &req_headers, &body_bytes);
    }

    let log_file_path = if state.file_enabled {
        write_request_file(&state.log_dir, &ts_str, short_id, &method.to_string(), &path_and_query, &req_headers, &body_bytes)
    } else {
        None
    };

    // ── 3. Forward to upstream ──────────────────────────────────────────────
    let upstream_url = format!("{}{}", state.upstream, path_and_query);

    let upstream_req = state
        .client
        .request(method, &upstream_url)
        .headers(forward_request_headers(&req_headers))
        .body(body_bytes);

    let upstream_resp = match upstream_req.send().await {
        Ok(resp) => resp,
        Err(e) => {
            let msg = format!("Upstream error: {}", e);
            tracing::error!("{}", msg);
            if state.console_enabled {
                eprintln!("{} {} {}", "  ERROR".red().bold(), short_id.dimmed(), msg.red());
            }
            return Err(StatusCode::BAD_GATEWAY);
        }
    };

    let resp_status = upstream_resp.status();
    let resp_headers = upstream_resp.headers().clone();
    let is_sse = resp_headers
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .map(|ct| ct.contains("text/event-stream"))
        .unwrap_or(false);

    // ── 4a. SSE streaming response ──────────────────────────────────────────
    if is_sse {
        let byte_stream = upstream_resp.bytes_stream();
        let events_collector: Arc<Mutex<Vec<Value>>> = Arc::new(Mutex::new(Vec::new()));
        let events_clone = events_collector.clone();
        let console_enabled = state.console_enabled;
        let file_enabled = state.file_enabled;
        let sid = short_id.to_string();
        let log_fp = log_file_path.clone();
        let resp_hdrs_clone = resp_headers.clone();
        let resp_status_code = resp_status.as_u16();
        let start_clone = start;

        if state.console_enabled {
            log_response_header_console(&sid, resp_status.as_u16(), true, &resp_headers);
        }

        let tee_stream = async_stream::stream! {
            let mut buf = BytesMut::new();
            let mut event_counter: usize = 0;
            let mut pinned = std::pin::pin!(byte_stream);

            while let Some(chunk_result) = pinned.next().await {
                match chunk_result {
                    Ok(chunk) => {
                        let raw = chunk.clone();
                        buf.extend_from_slice(&chunk);

                        // Parse complete SSE frames from the buffer
                        loop {
                            let buf_str = String::from_utf8_lossy(&buf);
                            if let Some(pos) = buf_str.find("\n\n") {
                                let frame = buf_str[..pos].to_string();
                                let consumed = pos + 2;
                                let _ = buf.split_to(consumed);

                                for line in frame.lines() {
                                    let line = line.trim();
                                    if let Some(event_type) = line.strip_prefix("event:") {
                                        let _ = event_type.trim();
                                        // event type is captured in the data parse below
                                    } else if let Some(data) = line.strip_prefix("data:") {
                                        let data = data.trim();
                                        event_counter += 1;
                                        if data == "[DONE]" {
                                            if console_enabled {
                                                eprintln!("  {} {} {}",
                                                    format!("SSE #{}", event_counter).yellow(),
                                                    "[DONE]".dimmed(),
                                                    ""
                                                );
                                            }
                                        } else if let Ok(parsed) = serde_json::from_str::<Value>(data) {
                                            if console_enabled {
                                                log_sse_event_console(event_counter, &parsed);
                                            }
                                            events_clone.lock().unwrap().push(parsed);
                                        } else {
                                            // Non-JSON data line
                                            if console_enabled {
                                                eprintln!("  {} {}",
                                                    format!("SSE #{}", event_counter).yellow(),
                                                    data.dimmed()
                                                );
                                            }
                                        }
                                    }
                                }
                            } else {
                                break;
                            }
                        }

                        yield Ok::<Bytes, std::io::Error>(raw);
                    }
                    Err(e) => {
                        tracing::error!("Upstream stream error: {}", e);
                        if console_enabled {
                            eprintln!("  {} {}", "STREAM ERROR:".red().bold(), e);
                        }
                        break;
                    }
                }
            }

            // Stream ended — finalize
            let duration_ms = start_clone.elapsed().as_millis();
            if console_enabled {
                eprintln!("{}", format!(
                    "  └─ Stream complete: {} events, {}ms",
                    event_counter, duration_ms
                ).dimmed());
                eprintln!();
            }

            if file_enabled {
                if let Some(fp) = &log_fp {
                    let events = events_clone.lock().unwrap().clone();
                    write_sse_response_to_file(
                        fp,
                        resp_status_code,
                        &resp_hdrs_clone,
                        &events,
                        duration_ms,
                    );
                }
            }
        };

        let body = Body::from_stream(tee_stream);
        let mut builder = Response::builder().status(resp_status.as_u16());
        for (name, value) in &resp_headers {
            builder = builder.header(name.as_str(), value.as_bytes());
        }
        return builder.body(body).map_err(|e| {
            tracing::error!("Failed to build SSE response: {}", e);
            StatusCode::INTERNAL_SERVER_ERROR
        });
    }

    // ── 4b. Non-streaming response ──────────────────────────────────────────
    let resp_body = upstream_resp.bytes().await.map_err(|e| {
        tracing::error!("Failed to read upstream response body: {}", e);
        StatusCode::BAD_GATEWAY
    })?;

    let duration_ms = start.elapsed().as_millis();

    if state.console_enabled {
        log_response_console(short_id, resp_status.as_u16(), &resp_headers, &resp_body, duration_ms);
    }

    if state.file_enabled {
        if let Some(fp) = &log_file_path {
            write_response_to_file(fp, resp_status.as_u16(), &resp_headers, &resp_body, duration_ms);
        }
    }

    let mut builder = Response::builder().status(resp_status.as_u16());
    for (name, value) in &resp_headers {
        builder = builder.header(name.as_str(), value.as_bytes());
    }
    builder.body(Body::from(resp_body)).map_err(|e| {
        tracing::error!("Failed to build response: {}", e);
        StatusCode::INTERNAL_SERVER_ERROR
    })
}

// ─── Header Utilities ────────────────────────────────────────────────────────

fn forward_request_headers(original: &HeaderMap) -> reqwest::header::HeaderMap {
    let mut headers = reqwest::header::HeaderMap::new();
    for (name, value) in original.iter() {
        let name_str = name.as_str().to_lowercase();
        // Skip hop-by-hop headers
        if name_str == "host" || name_str == "transfer-encoding" || name_str == "connection" {
            continue;
        }
        if let Ok(n) = reqwest::header::HeaderName::from_bytes(name.as_str().as_bytes()) {
            if let Ok(v) = reqwest::header::HeaderValue::from_bytes(value.as_ref()) {
                headers.insert(n, v);
            }
        }
    }
    headers
}

fn headers_to_json(headers: &reqwest::header::HeaderMap) -> Value {
    let map: serde_json::Map<String, Value> = headers
        .iter()
        .map(|(k, v)| {
            (
                k.as_str().to_string(),
                Value::String(v.to_str().unwrap_or("<binary>").to_string()),
            )
        })
        .collect();
    Value::Object(map)
}

fn axum_headers_to_json(headers: &HeaderMap) -> Value {
    let map: serde_json::Map<String, Value> = headers
        .iter()
        .map(|(k, v)| {
            (
                k.as_str().to_string(),
                Value::String(v.to_str().unwrap_or("<binary>").to_string()),
            )
        })
        .collect();
    Value::Object(map)
}

// ─── Console Logging ─────────────────────────────────────────────────────────

fn log_request_console(
    short_id: &str,
    timestamp: &str,
    method: &str,
    path: &str,
    headers: &HeaderMap,
    body: &[u8],
) {
    eprintln!();
    eprintln!(
        "{}",
        "  ══════════════════════════════════════════════════════════"
            .green()
    );
    eprintln!(
        "  {} {}  {}",
        format!("REQUEST {}", short_id).green().bold(),
        method.yellow().bold(),
        path.white().bold(),
    );
    eprintln!("  {}", timestamp.dimmed());
    eprintln!(
        "{}",
        "  ──────────────────────────────────────────────────────────"
            .green()
    );

    // Headers (selected, with redaction)
    for (name, value) in headers.iter() {
        let val_str = value.to_str().unwrap_or("<binary>");
        let display_val = maybe_redact(name.as_str(), val_str);
        eprintln!("  {}: {}", name.as_str().cyan(), display_val);
    }

    // Body
    if !body.is_empty() {
        eprintln!(
            "{}",
            "  ──────────────────────────────────────────────────────────"
                .green()
        );
        if let Ok(json) = serde_json::from_slice::<Value>(body) {
            let pretty = serde_json::to_string_pretty(&json).unwrap_or_default();
            for line in pretty.lines() {
                eprintln!("  {}", line);
            }
        } else {
            eprintln!("  ({} bytes, non-JSON)", body.len());
        }
    }
    eprintln!(
        "{}",
        "  ══════════════════════════════════════════════════════════"
            .green()
    );
}

fn log_response_header_console(
    short_id: &str,
    status: u16,
    is_sse: bool,
    headers: &reqwest::header::HeaderMap,
) {
    eprintln!();
    let status_color = if status < 300 {
        format!("{}", status).green()
    } else if status < 400 {
        format!("{}", status).yellow()
    } else {
        format!("{}", status).red()
    };

    let mode = if is_sse { "streaming" } else { "" };
    eprintln!(
        "{}",
        "  ──────────────────────────────────────────────────────────"
            .blue()
    );
    eprintln!(
        "  {} {}  {}  {}",
        format!("RESPONSE {}", short_id).blue().bold(),
        status_color.bold(),
        mode.dimmed(),
        ""
    );

    // Show a few key response headers
    for key in &["content-type", "x-request-id", "anthropic-ratelimit-requests-remaining"] {
        if let Some(val) = headers.get(*key) {
            if let Ok(v) = val.to_str() {
                eprintln!("  {}: {}", key.cyan(), v);
            }
        }
    }
    eprintln!(
        "{}",
        "  ──────────────────────────────────────────────────────────"
            .blue()
    );
}

fn log_response_console(
    short_id: &str,
    status: u16,
    headers: &reqwest::header::HeaderMap,
    body: &[u8],
    duration_ms: u128,
) {
    log_response_header_console(short_id, status, false, headers);

    if !body.is_empty() {
        if let Ok(json) = serde_json::from_slice::<Value>(body) {
            let pretty = serde_json::to_string_pretty(&json).unwrap_or_default();
            for line in pretty.lines() {
                eprintln!("  {}", line);
            }
        } else {
            eprintln!("  ({} bytes, non-JSON)", body.len());
        }
    }

    eprintln!("{}", format!("  └─ {}ms", duration_ms).dimmed());
    eprintln!();
}

fn log_sse_event_console(index: usize, event: &Value) {
    // Extract the event type for highlighting
    let event_type = event
        .get("type")
        .and_then(|t| t.as_str())
        .unwrap_or("unknown");

    // For content_block_delta, show a compact summary
    let summary = match event_type {
        "content_block_delta" => {
            if let Some(delta) = event.get("delta") {
                if let Some(text) = delta.get("text").and_then(|t| t.as_str()) {
                    let truncated = if text.len() > 80 {
                        format!("{}...", &text[..80])
                    } else {
                        text.to_string()
                    };
                    format!("text: {:?}", truncated)
                } else if delta.get("partial_json").is_some() {
                    "partial_json: ...".to_string()
                } else {
                    serde_json::to_string(delta).unwrap_or_default()
                }
            } else {
                String::new()
            }
        }
        "message_start" => {
            if let Some(msg) = event.get("message") {
                let model = msg.get("model").and_then(|m| m.as_str()).unwrap_or("?");
                let id = msg.get("id").and_then(|i| i.as_str()).unwrap_or("?");
                format!("id={}, model={}", id, model)
            } else {
                String::new()
            }
        }
        "content_block_start" => {
            if let Some(cb) = event.get("content_block") {
                let cb_type = cb.get("type").and_then(|t| t.as_str()).unwrap_or("?");
                let idx = event.get("index").and_then(|i| i.as_u64()).unwrap_or(0);
                format!("index={}, type={}", idx, cb_type)
            } else {
                String::new()
            }
        }
        "message_delta" => {
            if let Some(delta) = event.get("delta") {
                let stop = delta
                    .get("stop_reason")
                    .and_then(|s| s.as_str())
                    .unwrap_or("null");
                format!("stop_reason={}", stop)
            } else {
                String::new()
            }
        }
        _ => String::new(),
    };

    if summary.is_empty() {
        eprintln!(
            "  {} {}",
            format!("SSE #{:<4}", index).yellow(),
            event_type.white().bold(),
        );
    } else {
        eprintln!(
            "  {} {} {}",
            format!("SSE #{:<4}", index).yellow(),
            event_type.white().bold(),
            summary.dimmed(),
        );
    }
}

fn maybe_redact(header_name: &str, value: &str) -> String {
    let name = header_name.to_lowercase();
    if name == "authorization" || name == "x-api-key" {
        redact_token(value)
    } else {
        value.to_string()
    }
}

fn redact_token(val: &str) -> String {
    if val.len() > 16 {
        format!("{}...{}", &val[..12], &val[val.len() - 4..])
    } else if val.len() > 4 {
        format!("{}...", &val[..4])
    } else {
        "REDACTED".to_string()
    }
}

// ─── File Logging ────────────────────────────────────────────────────────────

fn safe_filename_ts(ts: &str) -> String {
    ts.replace(':', "-")
}

fn write_request_file(
    log_dir: &PathBuf,
    timestamp: &str,
    short_id: &str,
    method: &str,
    path: &str,
    headers: &HeaderMap,
    body: &[u8],
) -> Option<PathBuf> {
    std::fs::create_dir_all(log_dir).ok()?;

    let safe_ts = safe_filename_ts(timestamp);
    let filename = format!("{}_{}.json", safe_ts, short_id);
    let filepath = log_dir.join(filename);

    let body_json: Value = if body.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(body)
            .unwrap_or_else(|_| Value::String(String::from_utf8_lossy(body).to_string()))
    };

    let log_entry = serde_json::json!({
        "request_id": short_id,
        "timestamp": timestamp,
        "request": {
            "method": method,
            "path": path,
            "headers": axum_headers_to_json(headers),
            "body": body_json,
        },
        "response": null,
        "duration_ms": null,
    });

    let content = serde_json::to_string_pretty(&log_entry).ok()?;
    std::fs::write(&filepath, content).ok()?;
    Some(filepath)
}

fn write_response_to_file(
    filepath: &PathBuf,
    status: u16,
    headers: &reqwest::header::HeaderMap,
    body: &[u8],
    duration_ms: u128,
) {
    let Ok(content) = std::fs::read_to_string(filepath) else {
        return;
    };
    let Ok(mut entry) = serde_json::from_str::<Value>(&content) else {
        return;
    };

    let body_json: Value = if body.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(body)
            .unwrap_or_else(|_| Value::String(String::from_utf8_lossy(body).to_string()))
    };

    entry["response"] = serde_json::json!({
        "status": status,
        "headers": headers_to_json(headers),
        "is_streaming": false,
        "body": body_json,
    });
    entry["duration_ms"] = serde_json::json!(duration_ms);

    let _ = std::fs::write(filepath, serde_json::to_string_pretty(&entry).unwrap_or_default());
}

fn write_sse_response_to_file(
    filepath: &PathBuf,
    status: u16,
    headers: &reqwest::header::HeaderMap,
    events: &[Value],
    duration_ms: u128,
) {
    let Ok(content) = std::fs::read_to_string(filepath) else {
        return;
    };
    let Ok(mut entry) = serde_json::from_str::<Value>(&content) else {
        return;
    };

    entry["response"] = serde_json::json!({
        "status": status,
        "headers": headers_to_json(headers),
        "is_streaming": true,
        "event_count": events.len(),
        "events": events,
    });
    entry["duration_ms"] = serde_json::json!(duration_ms);

    let _ = std::fs::write(filepath, serde_json::to_string_pretty(&entry).unwrap_or_default());
}
