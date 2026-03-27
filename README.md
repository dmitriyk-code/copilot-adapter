# GitHub Copilot API Adapter for Claude Code

A standalone Rust binary that acts as an **OpenAI-compatible proxy** to GitHub Copilot's API. This adapter enables Claude Code users with GitHub Copilot subscriptions to leverage those subscriptions through the familiar OpenAI API interface.

## Features

- **GitHub OAuth device flow** — authenticate through your browser in seconds
- **Anthropic-compatible API** — `POST /v1/messages` for native Claude Code integration
- **OpenAI-compatible API** — `POST /v1/chat/completions`, `GET /v1/models`
- **SSE streaming** — real-time token-by-token responses
- **Experimental tool/function support** — opt-in prompt injection for tool calling (see below)
- **Automatic token management** — Copilot tokens refreshed 5 min before expiry
- **Secure credential storage** — OS keyring (macOS Keychain / Windows Credential Manager / Linux Secret Service) with encrypted file fallback
- **Background daemon** — runs as a background process on all platforms
- **Concurrent clients** — serves multiple simultaneous requests

## Quick Start

### 1. Install Claude Code

If you haven't already, install Claude Code from Anthropic:

```bash
npm install -g @anthropic-ai/claude-code
```

For more information, visit the [Claude Code documentation](https://docs.anthropic.com/en/docs/claude-code).

### 2. Install the Adapter

```bash
# From source
cargo install --path .

# Or build manually
cargo build --release
# Binary: target/release/copilot-adapter (or .exe on Windows)
```

### 3. Authenticate with GitHub

```bash
copilot-adapter auth
```

This starts the GitHub OAuth device flow:
1. Open the URL shown in your browser
2. Enter the displayed code
3. Authorize the application
4. Credentials are stored securely in your OS keyring

### 4. Start the Adapter

```bash
# Foreground mode
copilot-adapter start

# Background daemon
copilot-adapter start --daemon
```

The adapter starts listening on `http://127.0.0.1:6767` by default.

### 5. Configure Claude Code

Set the environment variables to point Claude Code at the adapter. Choose the format for your platform:

**Linux / macOS (bash/zsh):**

```bash
export ANTHROPIC_BASE_URL=http://127.0.0.1:6767
export ANTHROPIC_API_KEY=dummy  # Required by Claude Code but unused by the adapter
```

**Windows (Command Prompt):**

```cmd
set ANTHROPIC_BASE_URL=http://127.0.0.1:6767
set ANTHROPIC_API_KEY=dummy
```

**Windows (PowerShell):**

```powershell
$env:ANTHROPIC_BASE_URL = "http://127.0.0.1:6767"
$env:ANTHROPIC_API_KEY = "dummy"
```

> **Tip:** Add these to your shell profile (`.bashrc`, `.zshrc`, PowerShell `$PROFILE`) for persistence.

### 6. Run Claude Code

```bash
claude
```

Claude Code will automatically route requests through the adapter to GitHub Copilot.

## Commands

| Command | Description |
|---------|-------------|
| `copilot-adapter auth` | Authenticate with GitHub (device flow) |
| `copilot-adapter auth --force` | Re-authenticate, overwriting stored credentials |
| `copilot-adapter start` | Start the adapter in foreground |
| `copilot-adapter start --daemon` | Start as a background daemon |
| `copilot-adapter start -p 9090` | Start on a custom port |
| `copilot-adapter start --host 0.0.0.0` | Bind to all interfaces |
| `copilot-adapter start --log-level debug` | Enable debug logging |
| `copilot-adapter start --log-file /tmp/adapter.log` | Log to a file |
| `copilot-adapter start --experimental-tools` | Enable experimental tool/function support |
| `copilot-adapter status` | Check if the adapter is running |
| `copilot-adapter stop` | Stop the running daemon |
| `copilot-adapter logout` | Clear stored credentials |

## API Endpoints

### `GET /health`

Health check endpoint.

```bash
curl http://127.0.0.1:6767/health
# {"status": "ok"}
```

### `POST /v1/chat/completions`

OpenAI-compatible chat completions endpoint. Supports both streaming and non-streaming modes.

**Non-streaming:**

```bash
curl -X POST http://127.0.0.1:6767/v1/chat/completions \
  -H "Content-Type: application/json" \
  -d '{
    "model": "gpt-4",
    "messages": [{"role": "user", "content": "Hello!"}]
  }'
```

**Streaming:**

```bash
curl -X POST http://127.0.0.1:6767/v1/chat/completions \
  -H "Content-Type: application/json" \
  -d '{
    "model": "gpt-4",
    "messages": [{"role": "user", "content": "Hello!"}],
    "stream": true
  }'
```

**Supported request parameters:**

| Parameter | Type | Required | Description |
|-----------|------|----------|-------------|
| `model` | string | Yes | Model identifier (e.g., `gpt-4`, `gpt-4o`) |
| `messages` | array | Yes | Array of message objects (`role` + `content`) |
| `stream` | boolean | No | Enable SSE streaming (default: `false`) |
| `temperature` | number | No | Sampling temperature (0–2) |
| `max_tokens` | integer | No | Maximum tokens in the response |
| `top_p` | number | No | Nucleus sampling parameter |
| `n` | integer | No | Number of completions |
| `stop` | string/array | No | Stop sequences |
| `presence_penalty` | number | No | Presence penalty (-2 to 2) |
| `frequency_penalty` | number | No | Frequency penalty (-2 to 2) |

### `POST /v1/messages`

Anthropic-compatible messages endpoint. This is the **recommended endpoint for Claude Code** as it matches Claude's native API format. The adapter translates requests to OpenAI format internally before forwarding to Copilot.

**Non-streaming:**

```bash
curl -X POST http://127.0.0.1:6767/v1/messages \
  -H "Content-Type: application/json" \
  -H "x-api-key: dummy" \
  -H "anthropic-version: 2023-06-01" \
  -d '{
    "model": "claude-3-5-sonnet-20241022",
    "max_tokens": 1024,
    "messages": [{"role": "user", "content": "Hello!"}]
  }'
```

**Streaming:**

```bash
curl -X POST http://127.0.0.1:6767/v1/messages \
  -H "Content-Type: application/json" \
  -H "x-api-key: dummy" \
  -H "anthropic-version: 2023-06-01" \
  -d '{
    "model": "claude-3-5-sonnet-20241022",
    "max_tokens": 1024,
    "messages": [{"role": "user", "content": "Hello!"}],
    "stream": true
  }'
```

**Supported request parameters:**

| Parameter | Type | Required | Description |
|-----------|------|----------|-------------|
| `model` | string | Yes | Model identifier (mapped to Copilot model internally) |
| `messages` | array | Yes | Array of message objects (`role` + `content`) |
| `max_tokens` | integer | Yes | Maximum tokens in the response |
| `stream` | boolean | No | Enable SSE streaming (default: `false`) |
| `system` | string | No | System prompt |
| `temperature` | number | No | Sampling temperature (0–1) |
| `top_p` | number | No | Nucleus sampling parameter |
| `stop_sequences` | array | No | Stop sequences |

### `GET /v1/models`

List available models.

```bash
curl http://127.0.0.1:6767/v1/models
```

### `GET /v1/models/:model`

Get details for a specific model.

```bash
curl http://127.0.0.1:6767/v1/models/gpt-4
```

**Available models:** `gpt-4`, `gpt-4o`, `gpt-4-turbo`, `gpt-3.5-turbo`, `claude-3.5-sonnet`

## Experimental Tool/Function Support

The adapter supports **experimental tool/function calling** via prompt injection. Since GitHub Copilot's upstream API does not natively support the OpenAI `tools`/`functions` parameters, the adapter works around this by:

1. **Injecting** tool definitions into the system prompt as JSON
2. **Instructing** the model to respond with structured JSON when it wants to call a tool
3. **Parsing** tool calls from the model's text response
4. **Returning** them in the standard `tool_calls` format (OpenAI) or `tool_use` content blocks (Anthropic)

### Enabling Tools

Tool support is **disabled by default** and must be explicitly enabled:

```bash
copilot-adapter start --experimental-tools

# Or as a daemon
copilot-adapter start --daemon --experimental-tools
```

### Usage with Claude Code

Once the adapter is started with `--experimental-tools`, Claude Code's native tool use (file operations, bash commands, etc.) will work through the adapter:

```bash
export ANTHROPIC_BASE_URL=http://127.0.0.1:6767
export ANTHROPIC_API_KEY=dummy
claude  # Tools like bash, file read/write will work
```

### Usage with OpenAI-compatible Clients

Send standard OpenAI-format tool definitions in your requests:

```bash
curl -X POST http://127.0.0.1:6767/v1/chat/completions \
  -H "Content-Type: application/json" \
  -d '{
    "model": "gpt-4",
    "messages": [{"role": "user", "content": "What is the weather in London?"}],
    "tools": [{
      "type": "function",
      "function": {
        "name": "get_weather",
        "description": "Get the current weather",
        "parameters": {
          "type": "object",
          "properties": {
            "location": {"type": "string"}
          },
          "required": ["location"]
        }
      }
    }]
  }'
```

### Limitations

- **Opt-in only:** Requires `--experimental-tools` flag. Without it, requests with tools return HTTP 400.
- **Best-effort parsing:** Tool call parsing is based on regex/JSON extraction from text. The model may not always respond in the expected format. When parsing fails, the response gracefully degrades to a plain text message.
- **`tool_choice` limited:** Only `"auto"` behavior is supported. The `tool_choice` field is accepted but not enforced.
- **No `parallel_tool_calls`:** The `parallel_tool_calls` parameter is not supported.
- **Increased token usage:** Tool definitions are injected into the system prompt, increasing the token count.
- **Streaming support:** Tool calls in streaming responses are detected via buffering — the adapter buffers the full response, parses tool calls, then replays modified chunks.

## Configuration

### Port and Host

```bash
# Custom port
copilot-adapter start --port 9090

# Bind to all interfaces (for remote access — use with caution)
copilot-adapter start --host 0.0.0.0
```

### Logging

```bash
# Log levels: trace, debug, info, warn, error
copilot-adapter start --log-level debug

# Log to file (useful with --daemon)
copilot-adapter start --daemon --log-file /var/log/copilot-adapter.log

# Or use RUST_LOG environment variable
RUST_LOG=debug copilot-adapter start
```

The `--log-level` flag takes precedence over `RUST_LOG` when explicitly set.

### Credential Storage

Credentials are stored in priority order:

1. **OS Keyring** (preferred) — macOS Keychain, Windows Credential Manager, or Linux Secret Service (via D-Bus)
2. **Encrypted file** (fallback) — `~/.config/copilot-adapter/credentials.json` on Linux/macOS, `%APPDATA%\copilot-adapter\credentials.json` on Windows

To re-authenticate: `copilot-adapter auth --force`

To remove all credentials: `copilot-adapter logout`

## Architecture

```
Claude Code  ──→  copilot-adapter (localhost:6767)  ──→  GitHub Copilot API
                        │
                  ┌─────┴─────┐
                  │ Token Mgr  │  Auto-refresh Copilot tokens
                  │ Credential │  OS keyring / encrypted file
                  │ SSE Stream │  Real-time streaming support
                  └───────────┘
```

The adapter:
1. Accepts OpenAI-format requests on localhost
2. Authenticates with GitHub via stored OAuth token
3. Exchanges the GitHub token for a short-lived Copilot token (auto-refreshed)
4. Forwards the request to the Copilot API with required headers
5. Returns the response in OpenAI format

## Building from Source

### Prerequisites

- **Rust** 1.75 or later (`rustup` recommended)
- **Platform-specific:**
  - **Linux:** `libdbus-1-dev` and `libsecret-1-dev` (for keyring support)
  - **macOS:** Xcode command line tools
  - **Windows:** Visual Studio Build Tools

### Build

```bash
# Development build
cargo build

# Optimized release build
cargo build --release

# Cross-compilation examples
cargo build --release --target x86_64-pc-windows-msvc
cargo build --release --target x86_64-apple-darwin
cargo build --release --target x86_64-unknown-linux-musl
```

### Run Tests

```bash
# All tests
cargo test

# Unit tests only
cargo test --test unit

# Integration tests only
cargo test --test integration

# With output
cargo test -- --nocapture
```

## Troubleshooting

### "Not authenticated" error

Run `copilot-adapter auth` to authenticate. If you're already authenticated, try `copilot-adapter auth --force` to refresh credentials.

### "Adapter is already running"

Another instance is running. Stop it first:

```bash
copilot-adapter stop
```

If the process crashed, the stale PID file will be cleaned up automatically on the next `start` or `status` command.

### Connection refused

1. Verify the adapter is running: `copilot-adapter status`
2. Check the port matches your `OPENAI_API_BASE` setting
3. Ensure no firewall is blocking localhost connections

### Token refresh failures

The adapter auto-refreshes Copilot tokens 5 minutes before expiry. If you see refresh errors:

1. Check your GitHub Copilot subscription is active
2. Re-authenticate: `copilot-adapter auth --force`
3. Check logs for details: `copilot-adapter start --log-level debug`

### Keyring not available

On headless Linux systems, the OS keyring may not be available. The adapter falls back to encrypted file storage automatically. No action needed.

### Rate limiting (HTTP 429)

The adapter forwards rate limit responses from the Copilot API with the `Retry-After` header. Reduce request frequency or wait for the retry-after period.

### Debug logging

For detailed troubleshooting, enable debug or trace logging:

```bash
copilot-adapter start --log-level trace
# or
RUST_LOG=trace copilot-adapter start
```

## License

See LICENSE file for details.
