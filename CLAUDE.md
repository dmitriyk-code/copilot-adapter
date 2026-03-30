# GitHub Copilot API Adapter for Claude Code

## Overview

A standalone Rust binary (`copilot-adapter`) that acts as an **OpenAI-compatible proxy** to GitHub Copilot's API. It enables Claude Code users with GitHub Copilot subscriptions to leverage those subscriptions through the familiar OpenAI API interface.

## Key Features

- **GitHub OAuth device flow** authentication
- **OpenAI-compatible API** endpoints (`POST /v1/chat/completions`, `GET /v1/models`)
- **Anthropic-compatible API** endpoint (`POST /v1/messages`) with format translation
- **Model name normalization** — automatically translates Claude Code's versioned model names (e.g., `claude-haiku-4-5-20251001`) to GitHub Copilot's format (e.g., `claude-haiku-4.5`)
- **SSE streaming** support for real-time responses
- **Tool/function support** via prompt injection (always enabled)
- **Vision / image support** — translates Anthropic image blocks to OpenAI `image_url` format; document blocks gracefully skipped
- **Dynamic model discovery** — fetches available models from Copilot API with in-memory caching and fallback
- **Automatic token management** with refresh 5 min before expiry
- **Secure credential storage** via OS keyring (with encrypted file fallback)
- **Cross-platform daemon** operation (Windows/Linux/macOS)

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

## Project Structure

```
src/
├── main.rs              # Entry point, CLI handling
├── cli.rs               # CLI argument definitions (clap)
├── server.rs            # Axum HTTP server setup
├── error.rs             # Error types with OpenAI-compatible responses
├── model_mapper.rs      # Model name normalization (Claude Code format → Copilot format)
├── lib.rs               # Library exports
├── handlers/
│   ├── mod.rs
│   ├── chat.rs          # /v1/chat/completions (OpenAI format)
│   ├── messages.rs      # /v1/messages (Anthropic format)
│   ├── models.rs        # /v1/models endpoint (dynamic + fallback)
│   └── health.rs        # Health check
├── auth/
│   ├── mod.rs
│   ├── device_flow.rs   # GitHub OAuth device flow
│   └── token.rs         # Token manager with auto-refresh
├── copilot/
│   ├── mod.rs
│   ├── client.rs        # Copilot API client with SSE streaming + models fetch
│   ├── models_cache.rs  # In-memory models cache with TTL expiration
│   └── types.rs         # OpenAI request/response types
├── anthropic/
│   ├── mod.rs
│   └── types.rs         # Anthropic request/response types + translation
├── tools/
│   ├── mod.rs           # Tools module exports
│   ├── types.rs         # Tool/ToolCall type definitions
│   ├── injector.rs      # Prompt injection logic
│   └── parser.rs        # Tool call parsing from text responses
├── storage/
│   ├── mod.rs
│   ├── keyring.rs       # OS keyring storage
│   └── file.rs          # Encrypted file fallback
└── daemon/
    ├── mod.rs           # PID file management
    ├── unix.rs          # Unix daemonization
    └── windows.rs       # Windows background process
```

## Commands

| Command | Description |
|---------|-------------|
| `copilot-adapter auth` | Authenticate with GitHub (device flow) |
| `copilot-adapter auth --force` | Force re-authentication |
| `copilot-adapter start` | Start adapter in foreground |
| `copilot-adapter start --daemon` | Start as background daemon |
| `copilot-adapter start -p 9090` | Start on custom port |
| `copilot-adapter start --log-level debug` | Enable debug logging |
| `copilot-adapter start --log-level trace` | Enable trace logging (very verbose, logs full request/response JSON) |
| `copilot-adapter start --models-cache-ttl 600` | Set model list cache TTL (seconds) |
| `copilot-adapter start --static-models` | Use static model list (skip API) |
| `copilot-adapter status` | Check if adapter is running |
| `copilot-adapter stop` | Stop the running daemon |
| `copilot-adapter logout` | Clear stored credentials |

## Building

```bash
# Development build
cargo build

# Release build (optimized for size)
cargo build --release

# Run tests
cargo test
```

## Testing

- Unit tests: `cargo test --test unit`
- Integration tests: `cargo test --test integration`
- Manual E2E tests: See `docs/e2e-testing.md`

## Key Design Decisions

1. **Rust with axum**: Minimal binary, no runtime dependencies, excellent async support
2. **Single binary**: Easy distribution and installation
3. **OS keyring for tokens**: Platform-native secure storage
4. **Localhost-only by default**: Security - prevents external access without explicit opt-in
5. **SSE passthrough**: Copilot already returns OpenAI-compatible format

## API Endpoints

- `GET /health` - Health check
- `POST /v1/chat/completions` - OpenAI-format chat completions
- `POST /v1/messages` - Anthropic-format messages (translated internally)
- `GET /v1/models` - List available models
- `GET /v1/models/:model` - Get model details

## Important Files

- `DESIGN.md` - Full design document (architecture, API research, implementation details)
- `IMPLEMENTATION.plan.md` - Implementation plan with epics and tasks
- `DYNAMIC-MODELS.design.md` - Design document for dynamic models list feature (implemented)
- `DYNAMIC-MODELS.plan.md` - Implementation plan for dynamic models
- `TOOLS-SUPPORT.design.md` - Design document for tools/functions support (implemented)
- `TOOLS-SUPPORT.plan.md` - Implementation plan for tools support
- `docs/e2e-testing.md` - Manual end-to-end testing procedures

## Notes for Development

- **Trace logging**: When `--log-level trace` is enabled, the adapter logs the full request/response JSON at every transformation point: (1) incoming from Claude Code, (2) outgoing to GitHub Copilot API, (3) incoming from GitHub Copilot API, (4) outgoing to Claude Code. For streaming requests, each SSE chunk is logged individually. This is useful for debugging tool calls, model normalization, format translation, and streaming issues. Trace logs include structured fields: `direction` (INCOMING/OUTGOING), `source`/`destination` (Claude Code/GitHub Copilot API), `endpoint`, `format` (OpenAI/Anthropic), `mode` (streaming/non-streaming), and full JSON payloads.
- **Model name normalization**: The adapter automatically translates Claude Code's versioned model identifiers (e.g., `claude-haiku-4-5-20251001`) to GitHub Copilot's expected format (e.g., `claude-haiku-4.5`). This normalization happens in `src/model_mapper.rs` and is applied to all incoming requests at both the `/v1/chat/completions` and `/v1/messages` endpoints.
- **Dynamic models**: `/v1/models` fetches from Copilot API with in-memory caching (TTL-based via `ModelsCache` in `AppState`). Falls back to a static list on API errors. Controlled by `--models-cache-ttl` (default 300s) and `--static-models` flags.
- `ModelsCache` uses `tokio::sync::RwLock<Option<CacheEntry>>` with `Instant`-based TTL expiration
- `CopilotClient::fetch_models()` calls `https://api.githubcopilot.com/models` with standard Copilot headers
- `resolve_models()` in `src/handlers/models.rs` orchestrates cache → API fetch → fallback flow
- **Tools/functions support** is always enabled — tool definitions are injected into the system prompt; tool calls are parsed from model text responses
- The tools implementation lives in `src/tools/` (types, injector, parser)
- Tool call parsing is best-effort; malformed JSON is silently skipped (graceful degradation)
- `tool_choice` only supports `"auto"` behavior; `parallel_tool_calls` is not supported
- Copilot tokens expire in ~30 min; the adapter refreshes them proactively
- Required Copilot headers: `Copilot-Integration-Id`, `Editor-Version`, `Editor-Plugin-Version`
- All errors return OpenAI-compatible JSON format
