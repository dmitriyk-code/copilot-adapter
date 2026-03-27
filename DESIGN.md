# GitHub Copilot API Adapter for Claude Code - Design Document

## Context

Claude Code needs a way to connect to GitHub Copilot's API through an OpenAI-compatible interface. This adapter will allow Claude Code users who have GitHub Copilot subscriptions to leverage those subscriptions seamlessly. The adapter must be portable (Windows/Linux/macOS), high-performance, and support concurrent streaming connections.

## Executive Summary

Build a standalone Rust binary (`copilot-adapter`) that:
1. Authenticates with GitHub using OAuth device flow (sole auth method for simplicity)
2. Translates OpenAI-compatible API requests to GitHub Copilot API
3. Serves as a local HTTP server with SSE streaming support
4. Supports background daemon operation with simple start/stop commands
5. Configurable logging levels (error/warn/info/debug/trace via `--log-level` flag and `RUST_LOG` env)

---

## 1. API Research Findings

### 1.1 GitHub Copilot Authentication Flow

The authentication uses GitHub's OAuth device flow:

**Step 1: Request Device Code**
```
POST https://github.com/login/device/code
Content-Type: application/x-www-form-urlencoded

client_id=Iv1.b507a08c87ecfe98&scope=read:user
```

Response:
```json
{
  "device_code": "3584d83530557fdd1f46af8289938c8ef79f9dc5",
  "user_code": "WDJB-MJHT",
  "verification_uri": "https://github.com/login/device",
  "expires_in": 900,
  "interval": 5
}
```

**Step 2: Poll for Token**
```
POST https://github.com/login/oauth/access_token
Content-Type: application/x-www-form-urlencoded
Accept: application/json

client_id=Iv1.b507a08c87ecfe98&device_code=<device_code>&grant_type=urn:ietf:params:oauth:grant-type:device_code
```

**Step 3: Exchange for Copilot Token**
```
GET https://api.github.com/copilot_internal/v2/token
Authorization: Bearer <github_access_token>
```

Response includes a short-lived Copilot token (~30 min TTL).

### 1.2 GitHub Copilot Chat API

**Endpoint:**
```
POST https://api.githubcopilot.com/chat/completions
```

**Headers:**
```
Authorization: Bearer <copilot_token>
Content-Type: application/json
Accept: text/event-stream (for streaming)
X-Request-Id: <uuid>
Copilot-Integration-Id: vscode-chat
Editor-Version: vscode/1.85.0
Editor-Plugin-Version: copilot-chat/0.12.0
Openai-Organization: github-copilot
Openai-Intent: conversation-agent
```

**Request Format (OpenAI-compatible):**
```json
{
  "model": "gpt-4",
  "messages": [
    {"role": "system", "content": "You are a helpful assistant."},
    {"role": "user", "content": "Hello"}
  ],
  "stream": true,
  "temperature": 0.7,
  "max_tokens": 4096,
  "top_p": 1,
  "n": 1
}
```

**Streaming Response (SSE):**
```
data: {"id":"chatcmpl-xxx","object":"chat.completion.chunk","created":1234567890,"model":"gpt-4","choices":[{"index":0,"delta":{"role":"assistant"},"finish_reason":null}]}

data: {"id":"chatcmpl-xxx","object":"chat.completion.chunk","created":1234567890,"model":"gpt-4","choices":[{"index":0,"delta":{"content":"Hello"},"finish_reason":null}]}

data: {"id":"chatcmpl-xxx","object":"chat.completion.chunk","created":1234567890,"model":"gpt-4","choices":[{"index":0,"delta":{},"finish_reason":"stop"}]}

data: [DONE]
```

### 1.3 Models Available

GitHub Copilot provides access to:
- `gpt-4` (default for chat)
- `gpt-4-turbo` / `gpt-4o` (newer models)
- `claude-3.5-sonnet` (via Copilot Enterprise)
- Code-specific models for completions

---

## 2. Architecture Design

### 2.1 High-Level Architecture

```
+------------------------------------------------------------------+
|                        Claude Code                                |
|                   (OpenAI-compatible client)                      |
+------------------------------------------------------------------+
                              |
                              v HTTP/SSE
+------------------------------------------------------------------+
|                    Copilot Adapter (Rust)                         |
|  +---------------+  +---------------+  +------------------------+ |
|  |   HTTP        |  |   Token       |  |    Request             | |
|  |   Server      |--|   Manager     |--|    Translator          | |
|  |   (axum)      |  |               |  |                        | |
|  +---------------+  +---------------+  +------------------------+ |
|         |                 |                     |                 |
|         v                 v                     v                 |
|  +---------------+  +---------------+  +------------------------+ |
|  |   SSE         |  |   Secure      |  |    HTTP Client         | |
|  |   Streaming   |  |   Storage     |  |    (reqwest)           | |
|  +---------------+  +---------------+  +------------------------+ |
+------------------------------------------------------------------+
                              |
                              v HTTPS/SSE
+------------------------------------------------------------------+
|                   GitHub Copilot API                              |
|              api.githubcopilot.com                                |
+------------------------------------------------------------------+
```

### 2.2 Component Design

#### 2.2.1 CLI Interface

```
copilot-adapter <command>

Commands:
  start       Start the adapter server
    --port <PORT>       Port to listen on (default: 8787)
    --host <HOST>       Host to bind to (default: 127.0.0.1)
    --daemon            Run as background process
    --log-file <PATH>   Log file path (default: stderr)
    --log-level <LEVEL> Log level: error, warn, info, debug, trace (default: info)

  stop        Stop the background adapter

  status      Check if adapter is running

  auth        Authenticate with GitHub
    --force             Force re-authentication

  logout      Remove stored credentials

  version     Show version information
```

**Logging Configuration:**
- Default level: `info`
- Configurable via `--log-level` flag or `RUST_LOG` environment variable
- Uses `tracing-subscriber` with env-filter support

#### 2.2.2 HTTP Server (axum)

Endpoints to implement:

| Endpoint | Method | Description |
|----------|--------|-------------|
| `/v1/chat/completions` | POST | OpenAI-compatible chat completions |
| `/v1/models` | GET | List available models |
| `/v1/models/{model}` | GET | Get model details |
| `/health` | GET | Health check endpoint |

Note: No web dashboard - status monitoring via CLI and `/health` endpoint only.

#### 2.2.3 Token Manager

```rust
struct TokenManager {
    github_token: Option<String>,
    copilot_token: Option<CopilotToken>,
    refresh_handle: Option<JoinHandle<()>>,
}

struct CopilotToken {
    token: String,
    expires_at: DateTime<Utc>,
}

impl TokenManager {
    async fn get_valid_token(&self) -> Result<String>;
    async fn refresh_if_needed(&self) -> Result<()>;
    async fn authenticate(&self) -> Result<DeviceAuthFlow>;
}
```

#### 2.2.4 Streaming Handler

```rust
async fn handle_chat_completion(
    State(state): State<AppState>,
    Json(request): Json<ChatCompletionRequest>,
) -> Result<Response, AppError> {
    let token = state.token_manager.get_valid_token().await?;

    if request.stream {
        // Return SSE stream
        let stream = stream_copilot_response(token, request).await?;
        Ok(Sse::new(stream).into_response())
    } else {
        // Return complete response
        let response = fetch_copilot_response(token, request).await?;
        Ok(Json(response).into_response())
    }
}
```

---

## 3. Technology Stack

### 3.1 Core Dependencies

| Crate | Version | Purpose |
|-------|---------|---------|
| `tokio` | 1.x | Async runtime |
| `axum` | 0.7+ | HTTP server framework |
| `reqwest` | 0.12+ | HTTP client |
| `serde` / `serde_json` | 1.x | JSON serialization |
| `clap` | 4.x | CLI argument parsing |
| `tracing` | 0.1 | Logging/tracing |
| `keyring` | 2.x | Secure credential storage |
| `uuid` | 1.x | Request ID generation |
| `chrono` | 0.4 | Date/time handling |

### 3.2 Platform-Specific

| Crate | Platform | Purpose |
|-------|----------|---------|
| `daemonize` | Unix | Background process |
| `windows-service` | Windows | Windows service (optional) |

### 3.3 Cargo.toml Structure

```toml
[package]
name = "copilot-adapter"
version = "0.1.0"
edition = "2021"
rust-version = "1.75"

[dependencies]
tokio = { version = "1", features = ["full"] }
axum = { version = "0.7", features = ["tokio"] }
axum-extra = { version = "0.9", features = ["typed-header"] }
reqwest = { version = "0.12", features = ["json", "stream", "rustls-tls"], default-features = false }
serde = { version = "1", features = ["derive"] }
serde_json = "1"
clap = { version = "4", features = ["derive"] }
tracing = "0.1"
tracing-subscriber = { version = "0.3", features = ["env-filter"] }
keyring = "2"
uuid = { version = "1", features = ["v4"] }
chrono = { version = "0.4", features = ["serde"] }
thiserror = "1"
anyhow = "1"
futures = "0.3"
tokio-stream = "0.1"
async-stream = "0.3"
tower-http = { version = "0.5", features = ["cors", "trace"] }
bytes = "1"

[target.'cfg(unix)'.dependencies]
daemonize = "0.5"

[target.'cfg(windows)'.dependencies]
# Windows-specific deps if needed

[profile.release]
lto = true
codegen-units = 1
strip = true
opt-level = "z"
```

---

## 4. Detailed Implementation

### 4.1 Project Structure

```
copilot-adapter/
├── Cargo.toml
├── src/
│   ├── main.rs              # Entry point, CLI handling
│   ├── cli.rs               # CLI argument definitions
│   ├── server.rs            # HTTP server setup
│   ├── handlers/
│   │   ├── mod.rs
│   │   ├── chat.rs          # Chat completions handler
│   │   ├── models.rs        # Models endpoint
│   │   └── health.rs        # Health check
│   ├── auth/
│   │   ├── mod.rs
│   │   ├── device_flow.rs   # GitHub device flow
│   │   └── token.rs         # Token management
│   ├── copilot/
│   │   ├── mod.rs
│   │   ├── client.rs        # Copilot API client
│   │   └── types.rs         # Request/response types
│   ├── storage/
│   │   ├── mod.rs
│   │   └── keyring.rs       # Secure token storage
│   ├── daemon/
│   │   ├── mod.rs
│   │   ├── unix.rs          # Unix daemon
│   │   └── windows.rs       # Windows background
│   └── error.rs             # Error types
```

### 4.2 Authentication Flow Implementation

```rust
// src/auth/device_flow.rs

const GITHUB_DEVICE_CODE_URL: &str = "https://github.com/login/device/code";
const GITHUB_TOKEN_URL: &str = "https://github.com/login/oauth/access_token";
const COPILOT_TOKEN_URL: &str = "https://api.github.com/copilot_internal/v2/token";
const CLIENT_ID: &str = "Iv1.b507a08c87ecfe98";

pub struct DeviceFlowAuth {
    client: reqwest::Client,
}

impl DeviceFlowAuth {
    pub async fn initiate(&self) -> Result<DeviceCodeResponse> {
        let response = self.client
            .post(GITHUB_DEVICE_CODE_URL)
            .header("Accept", "application/json")
            .form(&[
                ("client_id", CLIENT_ID),
                ("scope", "read:user"),
            ])
            .send()
            .await?;

        Ok(response.json().await?)
    }

    pub async fn poll_for_token(
        &self,
        device_code: &str,
        interval: u64,
    ) -> Result<String> {
        loop {
            tokio::time::sleep(Duration::from_secs(interval)).await;

            let response = self.client
                .post(GITHUB_TOKEN_URL)
                .header("Accept", "application/json")
                .form(&[
                    ("client_id", CLIENT_ID),
                    ("device_code", device_code),
                    ("grant_type", "urn:ietf:params:oauth:grant-type:device_code"),
                ])
                .send()
                .await?;

            let result: TokenPollResponse = response.json().await?;

            match result {
                TokenPollResponse::Success { access_token } => {
                    return Ok(access_token);
                }
                TokenPollResponse::Pending => continue,
                TokenPollResponse::SlowDown => {
                    tokio::time::sleep(Duration::from_secs(5)).await;
                }
                TokenPollResponse::Error(e) => return Err(e.into()),
            }
        }
    }

    pub async fn get_copilot_token(&self, github_token: &str) -> Result<CopilotToken> {
        let response = self.client
            .get(COPILOT_TOKEN_URL)
            .bearer_auth(github_token)
            .send()
            .await?;

        Ok(response.json().await?)
    }
}
```

### 4.3 Streaming Implementation

```rust
// src/handlers/chat.rs

pub async fn chat_completions(
    State(state): State<Arc<AppState>>,
    Json(mut request): Json<ChatCompletionRequest>,
) -> Result<Response, AppError> {
    let copilot_token = state.token_manager.get_valid_token().await?;

    // Add required headers
    let request_id = Uuid::new_v4().to_string();

    if request.stream.unwrap_or(false) {
        let stream = create_streaming_response(
            &state.http_client,
            copilot_token,
            request,
            request_id,
        ).await?;

        Ok(Sse::new(stream)
            .keep_alive(KeepAlive::default())
            .into_response())
    } else {
        let response = create_blocking_response(
            &state.http_client,
            copilot_token,
            request,
            request_id,
        ).await?;

        Ok(Json(response).into_response())
    }
}

async fn create_streaming_response(
    client: &reqwest::Client,
    token: String,
    request: ChatCompletionRequest,
    request_id: String,
) -> Result<impl Stream<Item = Result<Event, Infallible>>> {
    let response = client
        .post("https://api.githubcopilot.com/chat/completions")
        .bearer_auth(&token)
        .header("Content-Type", "application/json")
        .header("Accept", "text/event-stream")
        .header("X-Request-Id", &request_id)
        .header("Copilot-Integration-Id", "vscode-chat")
        .header("Editor-Version", "vscode/1.85.0")
        .json(&request)
        .send()
        .await?;

    let stream = response.bytes_stream();

    Ok(async_stream::stream! {
        let mut buffer = String::new();

        for await chunk in stream {
            match chunk {
                Ok(bytes) => {
                    buffer.push_str(&String::from_utf8_lossy(&bytes));

                    // Process complete SSE messages
                    while let Some(pos) = buffer.find("\n\n") {
                        let message = buffer[..pos].to_string();
                        buffer = buffer[pos + 2..].to_string();

                        if let Some(data) = message.strip_prefix("data: ") {
                            if data == "[DONE]" {
                                yield Ok(Event::default().data("[DONE]"));
                            } else {
                                yield Ok(Event::default().data(data));
                            }
                        }
                    }
                }
                Err(e) => {
                    tracing::error!("Stream error: {}", e);
                    break;
                }
            }
        }
    })
}
```

### 4.4 Token Manager with Auto-Refresh

```rust
// src/auth/token.rs

pub struct TokenManager {
    github_token: RwLock<Option<String>>,
    copilot_token: RwLock<Option<CopilotToken>>,
    auth_client: DeviceFlowAuth,
    storage: Box<dyn TokenStorage>,
}

impl TokenManager {
    pub async fn get_valid_token(&self) -> Result<String> {
        // Check if we have a valid copilot token
        {
            let token = self.copilot_token.read().await;
            if let Some(ref t) = *token {
                if t.is_valid() {
                    return Ok(t.token.clone());
                }
            }
        }

        // Need to refresh
        self.refresh_copilot_token().await
    }

    async fn refresh_copilot_token(&self) -> Result<String> {
        let github_token = {
            let token = self.github_token.read().await;
            token.clone().ok_or(AuthError::NotAuthenticated)?
        };

        let new_token = self.auth_client
            .get_copilot_token(&github_token)
            .await?;

        let token_string = new_token.token.clone();

        *self.copilot_token.write().await = Some(new_token);

        Ok(token_string)
    }

    pub fn start_auto_refresh(self: Arc<Self>) -> JoinHandle<()> {
        tokio::spawn(async move {
            loop {
                // Refresh 5 minutes before expiry
                let sleep_duration = {
                    let token = self.copilot_token.read().await;
                    if let Some(ref t) = *token {
                        let until_expiry = t.expires_at - Utc::now();
                        let refresh_in = until_expiry - chrono::Duration::minutes(5);
                        refresh_in.to_std().unwrap_or(Duration::from_secs(60))
                    } else {
                        Duration::from_secs(60)
                    }
                };

                tokio::time::sleep(sleep_duration).await;

                if let Err(e) = self.refresh_copilot_token().await {
                    tracing::error!("Failed to refresh token: {}", e);
                }
            }
        })
    }
}
```

### 4.5 Background Process Management

```rust
// src/daemon/mod.rs

#[cfg(unix)]
pub use unix::*;
#[cfg(windows)]
pub use windows::*;

const PID_FILE: &str = ".copilot-adapter.pid";

pub fn get_pid_path() -> PathBuf {
    dirs::data_local_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(PID_FILE)
}

pub fn is_running() -> Option<u32> {
    let pid_path = get_pid_path();
    if !pid_path.exists() {
        return None;
    }

    let pid: u32 = std::fs::read_to_string(&pid_path)
        .ok()?
        .trim()
        .parse()
        .ok()?;

    // Check if process is actually running
    if process_exists(pid) {
        Some(pid)
    } else {
        let _ = std::fs::remove_file(&pid_path);
        None
    }
}

// src/daemon/unix.rs
#[cfg(unix)]
pub fn daemonize() -> Result<()> {
    use daemonize::Daemonize;

    let pid_path = get_pid_path();

    let daemon = Daemonize::new()
        .pid_file(&pid_path)
        .working_directory(".")
        .exit_action(|| println!("Adapter started in background"));

    daemon.start()?;
    Ok(())
}

#[cfg(unix)]
pub fn stop_daemon() -> Result<()> {
    if let Some(pid) = is_running() {
        unsafe {
            libc::kill(pid as i32, libc::SIGTERM);
        }
        let _ = std::fs::remove_file(get_pid_path());
        Ok(())
    } else {
        Err(anyhow::anyhow!("Adapter is not running"))
    }
}
```

---

## 5. Concurrent Client Support

### 5.1 Design for Multiple Clients

The adapter supports multiple concurrent clients through:

1. **Shared Token Manager**: Single `Arc<TokenManager>` shared across all handlers
2. **Connection Pooling**: reqwest's built-in connection pooling for upstream requests
3. **Async Streaming**: Each client gets independent SSE stream via tokio tasks

```rust
// src/server.rs

pub struct AppState {
    pub token_manager: Arc<TokenManager>,
    pub http_client: reqwest::Client,
}

pub fn create_router(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/v1/chat/completions", post(handlers::chat::chat_completions))
        .route("/v1/models", get(handlers::models::list_models))
        .route("/v1/models/:model", get(handlers::models::get_model))
        .route("/health", get(handlers::health::health_check))
        .layer(
            TraceLayer::new_for_http()
                .make_span_with(DefaultMakeSpan::new().level(Level::INFO))
        )
        .layer(
            CorsLayer::new()
                .allow_origin(Any)
                .allow_methods([Method::GET, Method::POST])
                .allow_headers(Any)
        )
        .with_state(state)
}
```

### 5.2 Request Isolation

Each request is handled independently:
- Request extraction and validation
- Token acquisition (shared, thread-safe)
- Independent upstream request
- Independent response streaming

---

## 6. Security Considerations

### 6.1 Token Storage

- **Primary**: OS keyring (Keychain on macOS, Credential Manager on Windows, Secret Service on Linux)
- **Fallback**: Encrypted file in user data directory
- **Never**: Plain text or environment variables for persistent storage

### 6.2 Network Security

- All upstream connections use HTTPS with certificate validation
- Local server binds to `127.0.0.1` by default (localhost only)
- Optional TLS for local connections (for reverse proxy setups)

### 6.3 Process Security

- PID file prevents multiple instances
- Graceful shutdown on SIGTERM/SIGINT
- Token cleared from memory on shutdown

---

## 7. API Compatibility Matrix

### 7.1 Supported OpenAI Endpoints

| Endpoint | Support | Notes |
|----------|---------|-------|
| `POST /v1/chat/completions` | Full | Streaming and non-streaming |
| `GET /v1/models` | Full | Returns Copilot-available models |
| `GET /v1/models/{model}` | Full | Model details |

### 7.2 Supported Request Parameters

| Parameter | Support | Notes |
|-----------|---------|-------|
| `model` | Full | Maps to Copilot models |
| `messages` | Full | System, user, assistant roles |
| `stream` | Full | SSE streaming |
| `temperature` | Full | 0.0 - 2.0 |
| `max_tokens` | Full | Context-dependent max |
| `top_p` | Full | Nucleus sampling |
| `n` | Partial | Only n=1 supported |
| `stop` | Full | Stop sequences |
| `presence_penalty` | Full | |
| `frequency_penalty` | Full | |
| `functions` | Not supported | Copilot limitation |
| `tools` | Not supported | Copilot limitation |

---

## 8. Error Handling

### 8.1 Error Types

```rust
// src/error.rs

#[derive(thiserror::Error, Debug)]
pub enum AppError {
    #[error("Authentication required")]
    NotAuthenticated,

    #[error("Token expired")]
    TokenExpired,

    #[error("GitHub API error: {0}")]
    GitHubError(String),

    #[error("Copilot API error: {0}")]
    CopilotError(String),

    #[error("Rate limited, retry after {0}s")]
    RateLimited(u64),

    #[error("Invalid request: {0}")]
    InvalidRequest(String),

    #[error("Internal error: {0}")]
    Internal(#[from] anyhow::Error),
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let (status, error_response) = match &self {
            AppError::NotAuthenticated => (
                StatusCode::UNAUTHORIZED,
                json!({"error": {"message": self.to_string(), "type": "authentication_error"}})
            ),
            AppError::RateLimited(secs) => (
                StatusCode::TOO_MANY_REQUESTS,
                json!({"error": {"message": self.to_string(), "type": "rate_limit_error", "retry_after": secs}})
            ),
            // ... other cases
        };

        (status, Json(error_response)).into_response()
    }
}
```

---

## 9. Testing Strategy

### 9.1 Unit Tests
- Token parsing and validation
- Request/response serialization
- Error mapping

### 9.2 Integration Tests
- Mock GitHub OAuth server
- Mock Copilot API responses
- Streaming behavior verification

### 9.3 End-to-End Tests
- Full authentication flow (requires manual GitHub login)
- Real Copilot API calls (requires active subscription)

---

## 10. Build & Distribution

### 10.1 Build Commands

```bash
# Development
cargo build

# Release (optimized)
cargo build --release

# Cross-compilation
cargo build --release --target x86_64-pc-windows-msvc
cargo build --release --target x86_64-apple-darwin
cargo build --release --target x86_64-unknown-linux-musl
```

### 10.2 Binary Size Optimization

Target: < 10MB release binary

- `lto = true` - Link-time optimization
- `codegen-units = 1` - Single codegen unit
- `opt-level = "z"` - Size optimization
- `strip = true` - Strip symbols
- rustls instead of native-tls (smaller, portable)

---

## 11. Usage Examples

### 11.1 First Time Setup

```bash
# Authenticate with GitHub
copilot-adapter auth

# Follow the prompts:
# Please visit: https://github.com/login/device
# Enter code: ABCD-1234
# Waiting for authorization...
# Successfully authenticated!

# Start the adapter
copilot-adapter start --daemon
# Adapter started on http://127.0.0.1:8787
```

### 11.2 Using with Claude Code

```bash
# Set environment variable
export OPENAI_API_BASE=http://127.0.0.1:8787/v1
export OPENAI_API_KEY=dummy  # Required but unused

# Or configure in Claude Code settings
```

### 11.3 Management Commands

```bash
# Check status
copilot-adapter status
# Adapter running on PID 12345, port 8787

# Stop the adapter
copilot-adapter stop

# View logs
copilot-adapter start --log-file /tmp/copilot.log
tail -f /tmp/copilot.log
```

---

## 12. Implementation Phases

### Phase 1: Core Infrastructure
- [x] Research complete
- [ ] Project setup with Cargo.toml
- [ ] CLI argument parsing
- [ ] Basic HTTP server

### Phase 2: Authentication
- [ ] GitHub device flow
- [ ] Token storage (keyring)
- [ ] Token refresh logic

### Phase 3: API Implementation
- [ ] Chat completions endpoint
- [ ] Streaming support
- [ ] Models endpoint

### Phase 4: Background Operation
- [ ] Unix daemonization
- [ ] Windows background process
- [ ] PID file management

### Phase 5: Polish
- [ ] Error handling
- [ ] Logging/tracing
- [ ] Documentation
- [ ] Tests

---

## 13. Verification Plan

After implementation, verify the adapter works correctly:

1. **Authentication Test**
   ```bash
   copilot-adapter auth
   # Should complete device flow successfully
   ```

2. **Server Start Test**
   ```bash
   copilot-adapter start
   curl http://127.0.0.1:8787/health
   # Should return {"status": "ok"}
   ```

3. **Models Test**
   ```bash
   curl http://127.0.0.1:8787/v1/models
   # Should return list of available models
   ```

4. **Chat Completion Test**
   ```bash
   curl -X POST http://127.0.0.1:8787/v1/chat/completions \
     -H "Content-Type: application/json" \
     -d '{"model": "gpt-4", "messages": [{"role": "user", "content": "Hello"}]}'
   ```

5. **Streaming Test**
   ```bash
   curl -X POST http://127.0.0.1:8787/v1/chat/completions \
     -H "Content-Type: application/json" \
     -d '{"model": "gpt-4", "messages": [{"role": "user", "content": "Hello"}], "stream": true}'
   # Should receive SSE events
   ```

6. **Concurrent Clients Test**
   - Open multiple terminal windows
   - Send concurrent requests
   - Verify all receive responses

---

## 14. Files to Create

| File | Purpose |
|------|---------|
| `Cargo.toml` | Project manifest |
| `src/main.rs` | Entry point |
| `src/cli.rs` | CLI definitions |
| `src/server.rs` | HTTP server |
| `src/handlers/mod.rs` | Handler module |
| `src/handlers/chat.rs` | Chat completions |
| `src/handlers/models.rs` | Models endpoint |
| `src/handlers/health.rs` | Health check |
| `src/auth/mod.rs` | Auth module |
| `src/auth/device_flow.rs` | OAuth flow |
| `src/auth/token.rs` | Token manager |
| `src/copilot/mod.rs` | Copilot module |
| `src/copilot/client.rs` | API client |
| `src/copilot/types.rs` | Type definitions |
| `src/storage/mod.rs` | Storage module |
| `src/storage/keyring.rs` | Keyring storage |
| `src/daemon/mod.rs` | Daemon module |
| `src/daemon/unix.rs` | Unix daemon |
| `src/daemon/windows.rs` | Windows daemon |
| `src/error.rs` | Error types |
| `README.md` | Documentation |

---

## Appendix A: Known Copilot API Quirks

1. **Token Expiration**: Copilot tokens expire after ~30 minutes; must refresh proactively
2. **Rate Limiting**: Copilot has undocumented rate limits; implement exponential backoff
3. **Model Names**: May need to map between OpenAI model names and Copilot-specific names
4. **Headers**: Some headers are required or Copilot returns 403
5. **User-Agent**: May need specific User-Agent format

## Appendix B: Alternative Approaches Considered

1. **Node.js Implementation**: Faster development but larger binary size and runtime dependency
2. **Go Implementation**: Good performance but less mature async ecosystem
3. **Python Implementation**: Excellent libraries but poor standalone distribution
4. **Rust chosen for**: Minimal binary, no runtime deps, excellent async support, strong type safety
