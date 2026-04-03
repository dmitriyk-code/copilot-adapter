# GitHub Copilot API Adapter for Claude Code

## Overview

A standalone Rust binary (`copilot-adapter`) that acts as an **Anthropic-to-Copilot proxy**. It enables Claude Code users with GitHub Copilot subscriptions to leverage those subscriptions by translating Anthropic API requests to GitHub Copilot's API format.

## Key Features

- **GitHub OAuth device flow** authentication
- **Anthropic-compatible API** endpoint (`POST /v1/messages`) with format translation to OpenAI internally
- **Model name normalization** — automatically translates Claude Code's versioned model names (e.g., `claude-haiku-4-5-20251001`) to GitHub Copilot's format (e.g., `claude-haiku-4.5`)
- **SSE streaming** support for real-time responses
- **Tool/function support** with native OpenAI function calling (progressive streaming) and automatic XML fallback
- **Vision / image support** — translates Anthropic image blocks to OpenAI `image_url` format; document blocks gracefully skipped
- **Dynamic model discovery** — fetches available models from Copilot API with in-memory caching and fallback
- **Automatic token management** with refresh 5 min before expiry
- **Secure credential storage** — platform-native encryption (DPAPI on Windows, OS keyring on macOS/Linux) stored in `~/.copilot-adapter/profiles/<name>/github-copilot.json`
- **Multi-instance profiles** — run concurrent instances via `--profile` / `-P` flag with independent ports and credentials
- **Cross-platform daemon** operation (Windows/Linux/macOS)

## Architecture

```
Claude Code  ──→  copilot-adapter (localhost:6767)  ──→  GitHub Copilot API
                        │
                  ┌─────┴─────┐
                  │ Token Mgr  │  Auto-refresh Copilot tokens
                  │ Credential │  platform-native encryption
                  │ SSE Stream │  Real-time streaming support
                  └───────────┘
```

## Project Structure

```
src/
├── main.rs              # Entry point, CLI handling
├── cli.rs               # CLI argument definitions (clap)
├── server.rs            # Axum HTTP server setup
├── error.rs             # Error types with structured error responses
├── model_mapper.rs      # Model name normalization (Claude Code format → Copilot format)
├── lib.rs               # Library exports
├── handlers/
│   ├── mod.rs
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
│   ├── parser.rs        # Tool call parsing from text responses
│   ├── translator.rs    # Anthropic → OpenAI tool definition translation
│   └── registry.rs      # Tool schema registry for parameter type coercion
├── streaming/
│   ├── mod.rs           # Streaming module exports
│   └── state.rs         # Streaming state machine (OpenAI → Anthropic SSE)
├── storage/
│   ├── mod.rs           # TokenStorage trait, factory function
│   ├── native.rs        # Platform-native credential storage (DPAPI / keyring)
│   ├── legacy.rs        # XOR migration reader for old credentials.json
│   ├── dpapi.rs         # Windows DPAPI encryption FFI bindings
│   ├── windows_credential.rs  # Windows credential helper
│   ├── keyring.rs       # Legacy standalone OS keyring storage (not used by NativeStorage; available for direct use/testing)
│   └── file.rs          # XOR-based file storage (legacy)
├── daemon/
│   ├── mod.rs           # PID file management
│   ├── status.rs        # Runtime status (status.json) read/write
│   ├── unix.rs          # Unix daemonization
│   └── windows.rs       # Windows background process
└── profile/
    ├── mod.rs           # ProfileManager: CRUD, port conflict detection
    ├── types.rs         # Profile struct and serialization
    └── migration.rs     # Auto-migration from flat dir / legacy temp files
```

## Commands

| Command | Description |
|---------|-------------|
| `copilot-adapter auth` | Authenticate with GitHub (device flow) |
| `copilot-adapter auth --force` | Force re-authentication |
| `copilot-adapter auth -P <name>` | Authenticate for a specific profile |
| `copilot-adapter start` | Start adapter in foreground |
| `copilot-adapter start --daemon` | Start as background daemon |
| `copilot-adapter start -p 9090` | Start on custom port |
| `copilot-adapter start --profile <name>` / `-P <name>` | Start with a named profile |
| `copilot-adapter start --log-level debug` | Enable debug logging |
| `copilot-adapter start --log-level trace` | Enable trace logging (very verbose, logs full request/response JSON) |
| `copilot-adapter start --models-cache-ttl 600` | Set model list cache TTL (seconds) |
| `copilot-adapter start --static-models` | Use static model list (skip API) |
| `copilot-adapter start --disable-native-tools` | Disable native OpenAI tools and force XML-based tool injection |
| `copilot-adapter status` | Check if adapter is running |
| `copilot-adapter status -P <name>` | Check status for a specific profile |
| `copilot-adapter status --all` | Show status for all profiles |
| `copilot-adapter stop` | Stop the running daemon |
| `copilot-adapter stop -P <name>` | Stop a specific profile's daemon |
| `copilot-adapter stop --all` | Stop all running daemons |
| `copilot-adapter logout` | Clear stored credentials |
| `copilot-adapter logout -P <name>` | Clear credentials for a specific profile |
| `copilot-adapter profiles list` | List all profiles |
| `copilot-adapter profiles create <name>` | Create a named profile |
| `copilot-adapter profiles delete <name>` | Delete a named profile |

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
3. **Platform-native credentials**: Automatic encryption via DPAPI (Windows) or OS keyring (macOS/Linux) in `~/.copilot-adapter/profiles/<name>/github-copilot.json`
4. **Localhost-only by default**: Security - prevents external access without explicit opt-in
5. **SSE passthrough**: Copilot already returns OpenAI-compatible format

## API Endpoints

- `GET /` - Root path (health probe, returns 200 OK)
- `GET /health` - Health check
- `POST /v1/messages` - Anthropic-format messages (Claude Code native)
- `POST /v1/messages/count_tokens` - Pre-flight token counting (tiktoken-rs)
- `GET /v1/models` - List available models
- `GET /v1/models/:model` - Get model details

## Important Files
- `DESIGN.md` - Full design document (architecture, API research, implementation details)
- `IMPLEMENTATION.plan.md` - Implementation plan with epics and tasks
- `DYNAMIC-MODELS.design.md` - Design document for dynamic models list feature (implemented)
- `DYNAMIC-MODELS.plan.md` - Implementation plan for dynamic models
- `TOOLS-SUPPORT.design.md` - **Deprecated** — original tools design (JSON format). See `DUAL-RESPONSES.design.md`
- `TOOLS-SUPPORT.plan.md` - **Deprecated** — original tools plan. See `DUAL-RESPONSES.plan.md`
- `DUAL-RESPONSES.design.md` - Design document for XML tool format migration and endpoint cleanup
- `DUAL-RESPONSES.plan.md` - Implementation plan for XML tool format migration
- `NATIVE-TOOLS-STREAMING.design.md` - Design document for native OpenAI tools and progressive streaming
- `NATIVE-TOOLS-STREAMING.plan.md` - Implementation plan for native tools and schema-aware parsing
- `CONSOLIDATED.plan.md` - Consolidated implementation plan for daemon auth, home dir storage, file-first credentials, and multi-instance profiles
- `docs/e2e-testing.md` - Manual end-to-end testing procedures
- `docs/known-issues.md` - Known issues and workarounds

## Major changes development process (features and bug fixes that introduce new concepts or touch multiple files / components)

- Create a design document first under docs/design
  - Use docs/design/DESIGN.template.md
- Create an implementation plan
  - Use docs/design/PLAN.template.md
- If the feature is in the docs/design/backlog.md, update the backlog file

## Notes for Development

- **Trace logging**: When `--log-level trace` is enabled, the adapter logs the full request/response data at every transformation point: (1) incoming from Claude Code, (2) outgoing to GitHub Copilot API, (3) incoming from GitHub Copilot API, (4) outgoing to Claude Code. For streaming requests, each SSE chunk is logged individually. This is useful for debugging tool calls, model normalization, format translation, and streaming issues. Trace logs include structured fields: `direction` (INCOMING/OUTGOING), `source`/`destination` (Claude Code/GitHub Copilot API), `endpoint`, `format` (Anthropic), `mode` (streaming/non-streaming), and full payloads.
- **Root path handler**: `GET /` and `HEAD /` return `{"status": "ok"}` with HTTP 200. This eliminates 404 log noise from Claude Code's health probes (`HEAD /`). No authentication required. Implementation in `src/handlers/health.rs`, route registered in `src/server.rs`.
- **Token counting**: `POST /v1/messages/count_tokens` provides pre-flight token counting using `tiktoken-rs` with `cl100k_base` BPE encoding. Returns `{"input_tokens": N}`. Used by Claude Code for context window management. Implementation in `src/token_counter.rs` (counting logic) and `src/handlers/count_tokens.rs` (HTTP handler). Performance target: <10ms for typical requests. Text accuracy: >95%. Images/documents use fixed estimates (~85 tokens each).
- **Model name normalization**: The adapter automatically translates Claude Code's versioned model identifiers (e.g., `claude-haiku-4-5-20251001`) to GitHub Copilot's expected format (e.g., `claude-haiku-4.5`). This normalization happens in `src/model_mapper.rs` and is applied to all incoming requests at the `/v1/messages` endpoint.
- **Dynamic models**: `/v1/models` fetches from Copilot API with in-memory caching (TTL-based via `ModelsCache` in `AppState`). Falls back to a static list on API errors. Controlled by `--models-cache-ttl` (default 300s) and `--static-models` flags.
- `ModelsCache` uses `tokio::sync::RwLock<Option<CacheEntry>>` with `Instant`-based TTL expiration
- `CopilotClient::fetch_models()` calls `https://api.githubcopilot.com/models` with standard Copilot headers
- `resolve_models()` in `src/handlers/models.rs` orchestrates cache → API fetch → fallback flow
- **Tools/functions support**: By default, tool definitions are passed natively to the Copilot API in OpenAI format for progressive streaming. Falls back to XML injection (injected into system prompt using XML format following the Anthropic Cookbook) if the upstream API doesn't support native tools. Use `--disable-native-tools` to always use XML mode.
- The tools implementation lives in `src/tools/` (types, injector, parser, translator, registry)
- Tool call parsing is best-effort; malformed XML is silently skipped (graceful degradation)
- `tool_choice` only supports `"auto"` behavior; `parallel_tool_calls` is not supported
- Copilot tokens expire in ~30 min; the adapter refreshes them proactively
- Required Copilot headers: `Copilot-Integration-Id`, `Editor-Version`, `Editor-Plugin-Version`
- All errors return structured JSON format
- **Native tools** (default): Tool definitions are passed to the Copilot API in OpenAI format and responses stream progressively. Falls back to XML injection if not supported. Use `--disable-native-tools` to force XML-only mode.
- **Tool name truncation**: OpenAI has a 64-character limit for function names. Long names (common with MCP tools like `mcp__codemogger__codemogger_search`) are truncated with a hash suffix and restored in responses. Implementation in `src/tools/translator.rs`.
- **Parameter types**: Native tools preserve parameter types from schemas. XML fallback path coerces string values to their schema-defined types (number, boolean, etc.) via `ToolRegistry` in `src/tools/registry.rs`.
- **Streaming state machine**: The `StreamingState` in `src/streaming/state.rs` incrementally translates OpenAI streaming chunks to Anthropic SSE events, handling content block transitions, tool call deltas, and tool name restoration.
- **Daemon authentication**: Both foreground and daemon modes follow the same auth flow. When `start --daemon` is used without credentials, the adapter runs the interactive device flow *before* daemonizing (the parent process still has terminal access). The old daemon-specific auth gate that refused to start has been removed.
- **Home directory storage**: All runtime state lives under `~/.copilot-adapter/`. Runtime status (PID, port, version, started_at) is stored in `status.json`; credentials in `github-copilot.json` (platform-native encryption: DPAPI on Windows, OS keyring on macOS/Linux). Legacy temp-dir PID files are detected as fallback. Implementation in `src/daemon/status.rs` (status read/write) and `src/storage/native.rs` (credentials).
- **Credential storage**: Platform-native credential encryption is always enabled. Credentials are stored in `~/.copilot-adapter/profiles/<name>/github-copilot.json` using DPAPI on Windows or OS keyring on macOS/Linux. Legacy XOR-encrypted credentials (`credentials.json`) are automatically migrated on first access.
- **Multi-instance profiles**: The `--profile` / `-P` flag selects a named profile. Each profile has its own directory under `~/.copilot-adapter/profiles/<name>/` containing `status.json` and `github-copilot.json`. The default profile name is `"default"`. Port conflict detection prevents two profiles from binding the same port. Profile management via `profiles list/create/delete` subcommand. Implementation in `src/profile/` (types, CRUD, migration).
- **Profile migration**: On first startup, the adapter auto-migrates from the flat `~/.copilot-adapter/` layout (status.json + credentials.json at root) to `profiles/default/`. Legacy temp-dir PID files are synthesized into status.json format. Migration is idempotent — skipped if `profiles/` directory already exists. Implementation in `src/profile/migration.rs`.
