# GitHub Copilot API Adapter for Claude Code

A standalone Rust binary that acts as an **Anthropic-to-Copilot proxy**. This adapter enables Claude Code users with GitHub Copilot subscriptions to leverage those subscriptions by translating Anthropic API requests to GitHub Copilot's API format.

## Features

- **GitHub OAuth device flow** — authenticate through your browser in seconds
- **Anthropic-compatible API** — `POST /v1/messages` for native Claude Code integration
- **Model discovery** — `GET /v1/models` with dynamic fetching and caching
- **SSE streaming** — real-time token-by-token responses
- **Vision / image support** — image uploads translated to OpenAI multimodal format (base64 and URL)
- **Tool/function support** — prompt injection for tool calling (see below)
- **Dynamic model discovery** — fetches available models from Copilot API with caching and fallback
- **Automatic token management** — Copilot tokens refreshed 5 min before expiry
- **Secure credential storage** — OS keyring (macOS Keychain / Windows Credential Manager / Linux Secret Service) with encrypted file fallback
- **Background daemon** — runs as a background process on all platforms
- **Concurrent clients** — serves multiple simultaneous requests

## Quick Start

### 1. Install

```bash
# From source
cargo install --path .

# Or build manually
cargo build --release
# Binary: target/release/copilot-adapter (or .exe on Windows)
```

### 2. Start the Adapter

```bash
copilot-adapter start
```

On first run, the adapter will:
1. Detect missing authentication and start the OAuth flow
2. Offer to open the GitHub authorization URL in your browser
3. Wait for you to authorize the application
4. Display configuration instructions for Claude Code

The adapter starts listening on `http://127.0.0.1:6767` by default.

> **Note:** You can still authenticate separately with `copilot-adapter auth` if you prefer.

### 3. Configure Claude Code

Choose one of these methods:

**Method A: Environment Variables (session)**

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

> **Tip:** Add these to your shell profile (`.bashrc`, `.zshrc`, PowerShell `$PROFILE`) for persistence across terminal sessions.

**Method B: Claude Code Settings (recommended, persistent)**

Create or edit `~/.claude/settings.json`:

```json
{
  "env": {
    "ANTHROPIC_BASE_URL": "http://127.0.0.1:6767",
    "ANTHROPIC_API_KEY": "dummy"
  }
}
```

For project-specific configuration, create `.claude/settings.json` in your project root.

Settings precedence (highest to lowest):
1. `<project>/.claude/settings.local.json` (gitignored, for personal overrides)
2. `<project>/.claude/settings.json` (committed, for team sharing)
3. `~/.claude/settings.json` (user-level defaults)

### 4. Run Claude Code

```bash
claude
```

Claude Code will automatically route requests through the adapter to GitHub Copilot.

## Commands

| Command | Description |
|---------|-------------|
| `copilot-adapter auth` | Authenticate with GitHub (device flow) |
| `copilot-adapter auth --force` | Re-authenticate, overwriting stored credentials |
| `copilot-adapter start` | Start adapter (auto-authenticates if needed) |
| `copilot-adapter start --daemon` | Start as background daemon (requires prior auth) |
| `copilot-adapter start --skip-auth` | Start without auto-authentication |
| `copilot-adapter start --quiet` | Start without displaying setup guidance |
| `copilot-adapter start -p 9090` | Start on a custom port |
| `copilot-adapter start --host 0.0.0.0` | Bind to all interfaces |
| `copilot-adapter start --log-level debug` | Enable debug logging |
| `copilot-adapter start --log-file /tmp/adapter.log` | Log to a file |
| `copilot-adapter start --models-cache-ttl 600` | Set model list cache TTL (seconds, default: 300) |
| `copilot-adapter start --static-models` | Use static model list (skip API fetch) |
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
| `messages` | array | Yes | Array of message objects (`role` + `content`). Content can be a string or an array of content blocks (`text`, `image`, `document`) for multimodal messages. See [Vision / Image Support](#vision--image-support). |
| `max_tokens` | integer | Yes | Maximum tokens in the response |
| `stream` | boolean | No | Enable SSE streaming (default: `false`) |
| `system` | string | No | System prompt |
| `temperature` | number | No | Sampling temperature (0–1) |
| `top_p` | number | No | Nucleus sampling parameter |
| `stop_sequences` | array | No | Stop sequences |

### `GET /v1/models`

List available models. By default, models are fetched dynamically from the Copilot API and cached in memory (TTL: 5 minutes). If the API is unreachable, the adapter falls back to a built-in static list.

```bash
curl http://127.0.0.1:6767/v1/models
```

### `GET /v1/models/:model`

Get details for a specific model.

```bash
curl http://127.0.0.1:6767/v1/models/gpt-4
```

**Dynamic models behaviour:**

- On first request (or after cache expires), the adapter fetches the model list from `https://api.githubcopilot.com/models`
- Subsequent requests within the cache TTL return the cached list without an API call
- If the Copilot API is unavailable, the adapter returns a static fallback list (gpt-4o, gpt-4, gpt-4-turbo, gpt-3.5-turbo)
- Use `--static-models` to disable dynamic fetching entirely

## Vision / Image Support

The adapter supports **multimodal (image) content** in the Anthropic `/v1/messages` endpoint. When Claude Code or any Anthropic-compatible client sends image content blocks, the adapter translates them to OpenAI's `image_url` format before forwarding to the Copilot API.

### Supported Content Types

| Content Block | Support | Details |
|---------------|---------|---------|
| `text` | ✅ Full | Passed through as-is |
| `image` (base64) | ✅ Full | Converted to `data:{media_type};base64,{data}` data URI |
| `image` (URL) | ✅ Full | URL passed through unchanged |
| `document` | ⚠️ Skipped | Not supported by OpenAI format; skipped with a warning log |
| `cache_control` | ✅ Accepted | Deserialized without error; not forwarded to Copilot API |

### Usage

Image uploads work automatically — no additional flags or configuration required. Use a vision-capable model (e.g., `gpt-4o`) for best results.

**Base64 image:**

```bash
curl -s -X POST http://127.0.0.1:6767/v1/messages \
  -H "Content-Type: application/json" \
  -H "x-api-key: dummy" \
  -H "anthropic-version: 2023-06-01" \
  -d '{
    "model": "gpt-4o",
    "max_tokens": 1024,
    "messages": [{
      "role": "user",
      "content": [
        {"type": "text", "text": "Describe this image."},
        {
          "type": "image",
          "source": {
            "type": "base64",
            "media_type": "image/png",
            "data": "<base64-encoded image data>"
          }
        }
      ]
    }]
  }'
```

**URL image:**

```bash
curl -s -X POST http://127.0.0.1:6767/v1/messages \
  -H "Content-Type: application/json" \
  -H "x-api-key: dummy" \
  -H "anthropic-version: 2023-06-01" \
  -d '{
    "model": "gpt-4o",
    "max_tokens": 1024,
    "messages": [{
      "role": "user",
      "content": [
        {"type": "text", "text": "What do you see?"},
        {
          "type": "image",
          "source": {
            "type": "url",
            "url": "https://example.com/photo.jpg"
          }
        }
      ]
    }]
  }'
```

### How It Works

1. The adapter receives an Anthropic-format request with `image` content blocks
2. Each `image` block is translated to an OpenAI `image_url` content block:
   - **Base64 sources** → `data:{media_type};base64,{data}` data URI
   - **URL sources** → URL passed through directly
3. `document` blocks are skipped with a warning log (OpenAI format has no equivalent)
4. `cache_control` metadata is accepted on any content block (prevents deserialization errors) but is not forwarded to the upstream API
5. The translated multimodal message is sent to the Copilot API

### Limitations

- **Document blocks not supported:** PDF and other document uploads are silently skipped. The adapter logs a warning with the document title (if provided) but continues processing the rest of the message.
- **`cache_control` not forwarded:** Anthropic's `cache_control` metadata is accepted to prevent errors but has no effect on the upstream Copilot API.
- **Model must support vision:** Use a model with vision capabilities (e.g., `gpt-4o`). Non-vision models may ignore or error on image content.

## Tool/Function Support

The adapter supports **tool/function calling** via prompt injection. Since GitHub Copilot's upstream API does not natively support the Anthropic `tools` parameter, the adapter works around this by:

1. **Injecting** tool definitions into the system prompt as XML (following the Anthropic Cookbook format)
2. **Instructing** the model to respond with structured XML when it wants to call a tool
3. **Parsing** tool calls from `<function_calls>` XML blocks in the model's text response
4. **Returning** them as `tool_use` content blocks (Anthropic format)

### Usage with Claude Code

Claude Code's native tool use (file operations, bash commands, etc.) works automatically through the adapter:

```bash
export ANTHROPIC_BASE_URL=http://127.0.0.1:6767
export ANTHROPIC_API_KEY=dummy
claude  # Tools like bash, file read/write will work
```

### Usage with Anthropic-compatible Clients

Send standard Anthropic-format tool definitions in your requests:

```bash
curl -X POST http://127.0.0.1:6767/v1/messages \
  -H "Content-Type: application/json" \
  -H "x-api-key: dummy" \
  -H "anthropic-version: 2023-06-01" \
  -d '{
    "model": "claude-3-5-sonnet-20241022",
    "max_tokens": 1024,
    "messages": [{"role": "user", "content": "What is the weather in London?"}],
    "tools": [{
      "name": "get_weather",
      "description": "Get the current weather",
      "input_schema": {
        "type": "object",
        "properties": {
          "location": {"type": "string"}
        },
        "required": ["location"]
      }
    }]
  }'
```

### Limitations

- **Best-effort parsing:** Tool call parsing is based on XML extraction from text. The model may not always respond in the expected format. When parsing fails, the response gracefully degrades to a plain text message.
- **`tool_choice` limited:** Only `"auto"` behavior is supported. The `tool_choice` field is accepted but not enforced.
- **No `parallel_tool_calls`:** The `parallel_tool_calls` parameter is not supported.
- **Increased token usage:** Tool definitions are injected into the system prompt, increasing the token count.
- **Streaming support:** Tool calls in streaming responses are detected via buffering — the adapter buffers the full response, parses tool calls, then replays modified chunks.

### Debugging Tool Issues

If web search, web fetch, or other tools aren't working as expected, see the comprehensive debugging guide:

**→ [docs/debugging-tool-calls.md](docs/debugging-tool-calls.md)**

Quick start:
```bash
# Linux/macOS
./debug-responses.sh

# Windows
debug-responses.bat
```

This will run the adapter with trace-level logging to capture:
- What model is being requested
- What tools are being injected
- The raw response content from Copilot
- Whether tool calls are being parsed

See the debugging guide for how to interpret the logs and troubleshoot common issues.

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

### Dynamic Models

By default, the adapter fetches the model list from the Copilot API and caches it in memory. You can configure this behaviour:

```bash
# Set cache TTL to 10 minutes (default: 300 seconds / 5 minutes)
copilot-adapter start --models-cache-ttl 600

# Disable caching (always fetch fresh)
copilot-adapter start --models-cache-ttl 0

# Use the built-in static model list (no API calls for /v1/models)
copilot-adapter start --static-models
```

| Flag | Default | Description |
|------|---------|-------------|
| `--models-cache-ttl <seconds>` | `300` | How long to cache the model list (0 = no caching) |
| `--static-models` | `false` | Always return the built-in static list; skip Copilot API calls |

When the Copilot API is unreachable (network error, auth failure, HTTP error), the adapter logs a warning and returns the static fallback list. This ensures the `/v1/models` endpoint never fails.

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
1. Accepts Anthropic-format requests on localhost
2. Authenticates with GitHub via stored OAuth token
3. Exchanges the GitHub token for a short-lived Copilot token (auto-refreshed)
4. Translates requests to OpenAI format and forwards to the Copilot API with required headers
5. Translates responses back to Anthropic format and returns them

## Building from Source

### Prerequisites

- **Rust** 1.75 or later
  - Install via rustup (recommended):
    ```bash
    # Linux/macOS
    curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh

    # Windows
    # Download and run: https://rustup.rs/
    ```
  - Or install cargo directly:
    ```bash
    # Debian/Ubuntu
    sudo apt install cargo

    # Fedora/RHEL
    sudo dnf install cargo

    # Arch Linux
    sudo pacman -S rust

    # macOS (Homebrew)
    brew install rust

    # Windows (Chocolatey)
    choco install rust
    ```
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

### Auto-authentication not working in daemon mode

Daemon mode cannot perform interactive authentication. Run `copilot-adapter auth`
first, or start in foreground mode (`copilot-adapter start` without `--daemon`)
to authenticate interactively.

### Browser doesn't open during auth

The adapter waits 10 seconds for you to press Enter before opening the browser.
If your system doesn't support automatic browser opening, copy the URL manually.
On headless systems, the browser launch is skipped automatically.

### "Adapter is already running"

Another instance is running. Stop it first:

```bash
copilot-adapter stop
```

If the process crashed, the stale PID file will be cleaned up automatically on the next `start` or `status` command.

### Connection refused

1. Verify the adapter is running: `copilot-adapter status`
2. Check the port matches your `ANTHROPIC_BASE_URL` setting
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

## Known Issues

See [docs/known-issues.md](./docs/known-issues.md) for information about:
- Multiple responses when using Claude Code

## License

See LICENSE file for details.
