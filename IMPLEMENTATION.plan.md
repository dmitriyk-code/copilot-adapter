# GitHub Copilot API Adapter for Claude Code — Implementation Plan

**Status:** Draft
**Date:** 2026-03-26
**Based on:** [DESIGN.md](./DESIGN.md)

---

## Executive Summary

This plan implements the GitHub Copilot API Adapter as described in `DESIGN.md` — a standalone Rust binary that acts as an OpenAI-compatible proxy to GitHub Copilot's API. The adapter enables Claude Code users with GitHub Copilot subscriptions to leverage those subscriptions through the familiar OpenAI API interface. Key features include OAuth device flow authentication, secure token management with auto-refresh, SSE streaming support for real-time responses, and cross-platform background daemon operation. The implementation is organized into 7 epics covering core infrastructure, authentication, API implementation, streaming, background operation, error handling, and testing.

---

## Background

### Current State

No adapter exists. Claude Code users with GitHub Copilot subscriptions cannot use those subscriptions with Claude Code's OpenAI-compatible client interface.

### Target State

A fully functional adapter binary that:
- Authenticates via GitHub OAuth device flow
- Stores credentials securely in OS keyring
- Serves OpenAI-compatible endpoints on localhost
- Translates requests to GitHub Copilot API
- Supports concurrent streaming clients
- Runs as a background daemon

---

## Problem Statement

Claude Code requires an OpenAI-compatible API endpoint, but GitHub Copilot uses a proprietary authentication and API format:

1. **Authentication mismatch** — Copilot requires GitHub OAuth → Copilot token exchange, not simple API keys
2. **Token lifecycle** — Copilot tokens expire in ~30 minutes and must be refreshed proactively
3. **Header requirements** — Copilot API requires specific headers (`Copilot-Integration-Id`, `Editor-Version`, etc.)
4. **No direct compatibility** — While Copilot's request/response format is similar to OpenAI, the authentication and endpoint differ

---

## Goals and Non-Goals

### Goals

| ID | Goal | Success Criteria |
|----|------|------------------|
| G1 | Implement GitHub OAuth device flow authentication | User can authenticate via browser, token stored securely |
| G2 | Implement token management with auto-refresh | Tokens refresh 5 min before expiry; no request failures due to expired tokens |
| G3 | Implement OpenAI-compatible `/v1/chat/completions` endpoint | Requests with `stream: false` return complete responses |
| G4 | Implement SSE streaming support | Requests with `stream: true` return real-time SSE events |
| G5 | Implement `/v1/models` endpoints | Return list of available Copilot models |
| G6 | Implement cross-platform daemon operation | `start --daemon`, `stop`, `status` commands work on Windows/Linux/macOS |
| G7 | Support concurrent clients | Multiple simultaneous requests handled correctly |

### Non-Goals

| ID | Non-Goal | Rationale |
|----|----------|-----------|
| NG1 | Web dashboard UI | CLI and `/health` endpoint sufficient for status monitoring |
| NG2 | TLS for local server | Localhost-only; reverse proxy can add TLS if needed |
| NG3 | Support for `tools`/`functions` | Copilot limitation; not supported upstream |
| NG4 | Multiple user profiles | Single user per machine sufficient for initial version |
| NG5 | Windows service mode | Background process sufficient; service adds complexity |

---

## Requirements

### Functional Requirements

| ID | Requirement | Source |
|----|-------------|--------|
| FR1 | `copilot-adapter auth` initiates GitHub device flow | Design §11.1 |
| FR2 | Credentials stored in OS keyring with encrypted file fallback | Design §6.1 |
| FR3 | `POST /v1/chat/completions` accepts OpenAI-format requests | Design §2.2.2 |
| FR4 | Streaming responses use SSE format with `data:` prefix | Design §4.3 |
| FR5 | Token auto-refreshes 5 minutes before expiry | Design §4.4 |
| FR6 | `start --daemon` runs adapter in background | Design §2.2.1 |
| FR7 | `stop` terminates background adapter via PID file | Design §4.5 |
| FR8 | `/health` endpoint returns `{"status": "ok"}` | Design §2.2.2 |
| FR9 | Bind to `127.0.0.1` by default (localhost only) | Design §6.2 |
| FR10 | Support `--port`, `--host`, `--log-level` flags | Design §2.2.1 |

### Non-Functional Requirements

| ID | Requirement | Target |
|----|-------------|--------|
| NFR1 | Release binary size | < 10 MB |
| NFR2 | Response latency overhead | < 50 ms |
| NFR3 | Concurrent client support | ≥ 100 simultaneous streams |
| NFR4 | Memory usage | < 50 MB idle, < 100 MB under load |
| NFR5 | Cross-platform support | Windows, Linux, macOS |

---

## Proposed Architecture

### Component Overview

```
┌─────────────────────────────────────────────────────────────────┐
│                      copilot-adapter                             │
│                                                                 │
│  ┌───────────┐   ┌───────────────┐   ┌────────────────────────┐ │
│  │  CLI      │   │ HTTP Server   │   │ Token Manager          │ │
│  │  (clap)   │   │ (axum)        │   │ • GitHub token         │ │
│  └───────────┘   └───────────────┘   │ • Copilot token        │ │
│                         │            │ • Auto-refresh         │ │
│                         ▼            └────────────────────────┘ │
│  ┌───────────────────────────────┐              │               │
│  │ Handlers                       │              │               │
│  │ • /v1/chat/completions        │◄─────────────┘               │
│  │ • /v1/models                  │                              │
│  │ • /health                     │                              │
│  └───────────────────────────────┘                              │
│                   │                                             │
│                   ▼                                             │
│  ┌───────────────────────────────┐   ┌───────────────────────┐ │
│  │ Copilot Client (reqwest)      │   │ Token Storage         │ │
│  │ • Request translation         │   │ • Keyring (primary)   │ │
│  │ • SSE parsing                 │   │ • File (fallback)     │ │
│  └───────────────────────────────┘   └───────────────────────┘ │
│                   │                                             │
│  ┌───────────────────────────────┐                              │
│  │ Daemon Manager                │                              │
│  │ • Unix: daemonize crate       │                              │
│  │ • Windows: background process │                              │
│  │ • PID file management         │                              │
│  └───────────────────────────────┘                              │
└─────────────────────────────────────────────────────────────────┘
                            │
                            ▼ HTTPS
┌─────────────────────────────────────────────────────────────────┐
│                    GitHub Copilot API                            │
│           api.githubcopilot.com/chat/completions                 │
└─────────────────────────────────────────────────────────────────┘
```

### Key Design Decisions

| Decision | Rationale |
|----------|-----------|
| **Rust with axum** | Minimal binary, no runtime deps, excellent async support |
| **Single binary** | Easy distribution and installation |
| **OS keyring for tokens** | Platform-native secure storage |
| **Localhost-only by default** | Security: prevents external access without explicit opt-in |
| **SSE passthrough** | Minimal transformation; Copilot already returns OpenAI-compatible format |

---

## Dependencies

### External Dependencies

| Crate | Version | Purpose |
|-------|---------|---------|
| `tokio` | 1.x | Async runtime |
| `axum` | 0.7+ | HTTP server |
| `reqwest` | 0.12+ | HTTP client with streaming |
| `serde` / `serde_json` | 1.x | Serialization |
| `clap` | 4.x | CLI parsing |
| `tracing` / `tracing-subscriber` | 0.1 / 0.3 | Logging |
| `keyring` | 2.x | Secure credential storage |
| `uuid` | 1.x | Request ID generation |
| `chrono` | 0.4 | Timestamp handling |
| `thiserror` / `anyhow` | 1.x | Error handling |
| `daemonize` | 0.5 | Unix daemon (cfg(unix)) |

### Sequencing Constraints

1. Epic 1 (Core Infrastructure) must complete before all other epics
2. Epic 2 (Authentication) depends on Epic 1
3. Epic 3 (API Implementation) depends on Epics 1, 2
4. Epic 4 (Streaming) depends on Epic 3
5. Epic 5 (Daemon) depends on Epic 1
6. Epic 6 (Error Handling) can proceed in parallel with Epics 3-5
7. Epic 7 (Testing) depends on all other epics

---

## Impact Analysis

### Files Created

| File Path | Purpose |
|-----------|---------|
| `Cargo.toml` | Project manifest with all dependencies |
| `src/main.rs` | Entry point, CLI dispatch |
| `src/cli.rs` | CLI argument definitions (clap) |
| `src/server.rs` | Axum HTTP server setup |
| `src/handlers/mod.rs` | Handler module exports |
| `src/handlers/chat.rs` | Chat completions endpoint |
| `src/handlers/models.rs` | Models list/get endpoints |
| `src/handlers/health.rs` | Health check endpoint |
| `src/auth/mod.rs` | Auth module exports |
| `src/auth/device_flow.rs` | GitHub OAuth device flow |
| `src/auth/token.rs` | Token manager with auto-refresh |
| `src/copilot/mod.rs` | Copilot module exports |
| `src/copilot/client.rs` | Copilot API client |
| `src/copilot/types.rs` | Request/response types |
| `src/storage/mod.rs` | Storage module exports |
| `src/storage/keyring.rs` | Keyring-based token storage |
| `src/daemon/mod.rs` | Daemon module exports |
| `src/daemon/unix.rs` | Unix daemonization |
| `src/daemon/windows.rs` | Windows background process |
| `src/error.rs` | Custom error types |
| `tests/unit/mod.rs` | Unit test module |
| `tests/unit/token_tests.rs` | Token parsing tests |
| `tests/unit/types_tests.rs` | Serialization tests |
| `tests/integration/mod.rs` | Integration test module |
| `tests/integration/auth_tests.rs` | Auth flow tests |
| `tests/integration/chat_tests.rs` | Chat completion tests |
| `tests/integration/streaming_tests.rs` | SSE streaming tests |

---

## Risks and Mitigations

| # | Risk | Likelihood | Impact | Mitigation |
|---|------|-----------|--------|------------|
| R1 | Copilot API changes without notice | Medium | High | Version-pin editor headers; monitor for 403/401 errors |
| R2 | Rate limiting undocumented | Medium | Medium | Implement exponential backoff; surface retry-after headers |
| R3 | Keyring unavailable on some Linux distros | Low | Medium | Fall back to encrypted file storage |
| R4 | Token refresh race conditions | Medium | Medium | Use RwLock with careful lock ordering |
| R5 | SSE parsing edge cases | Low | Medium | Comprehensive integration tests with real API |

---

## Implementation Phases

### Phase 1: Foundation (Epics 1-2)
**Exit Criteria:** Project compiles; CLI parses arguments; `auth` command completes device flow; token stored in keyring.

### Phase 2: Core API (Epics 3-4)
**Exit Criteria:** `/v1/chat/completions` works for both streaming and non-streaming; `/v1/models` returns model list; concurrent requests handled.

### Phase 3: Operations (Epics 5-6)
**Exit Criteria:** `start --daemon`, `stop`, `status` work on all platforms; errors return OpenAI-compatible JSON; logging configurable.

### Phase 4: Quality (Epic 7)
**Exit Criteria:** Unit tests >80% coverage; integration tests pass with mock servers; manual E2E tests documented.

---

## Implementation Plan

### Epic 1: Core Infrastructure

**Goal:** Set up the Rust project structure, CLI framework, and basic HTTP server skeleton.

**Prerequisites:** None (foundational epic)

**Status:** DONE

**Tasks:**

| Task ID | Type | Description | Files | Status |
|---------|------|-------------|-------|--------|
| E1-T1 | IMPL | Create `Cargo.toml` with all dependencies from Design §3.3; configure release profile for size optimization | `Cargo.toml` | DONE |
| E1-T2 | IMPL | Create `src/main.rs` entry point with tokio async main, dispatch to CLI commands | `src/main.rs` | DONE |
| E1-T3 | IMPL | Create `src/cli.rs` with clap derive macros defining all commands and flags from Design §2.2.1 | `src/cli.rs` | DONE |
| E1-T4 | IMPL | Create `src/server.rs` with axum Router setup, placeholder routes, CORS and tracing layers | `src/server.rs` | DONE |
| E1-T5 | IMPL | Create `src/handlers/mod.rs` exporting handler modules | `src/handlers/mod.rs` | DONE |
| E1-T6 | IMPL | Create `src/handlers/health.rs` returning `{"status": "ok"}` | `src/handlers/health.rs` | DONE |
| E1-T7 | IMPL | Create `src/error.rs` with `AppError` enum and `IntoResponse` impl per Design §8.1 | `src/error.rs` | DONE |
| E1-T8 | TEST | Unit test: CLI parses all commands and flags correctly | `tests/unit/cli_tests.rs` | DONE |
| E1-T9 | TEST | Integration test: server starts, `/health` returns 200 OK | `tests/integration/server_tests.rs` | DONE |

**Acceptance Criteria:**
- [x] `cargo build` compiles without errors
- [x] `cargo run -- --help` shows all commands and flags
- [x] `cargo run -- start` starts server on default port
- [x] `curl localhost:8787/health` returns `{"status": "ok"}`

---

### Epic 2: Authentication System

**Goal:** Implement GitHub OAuth device flow, token exchange, and secure storage with keyring.

**Prerequisites:** Epic 1

**Status:** DONE (review fixes v2 applied)

**Tasks:**

| Task ID | Type | Description | Files | Status |
|---------|------|-------------|-------|--------|
| E2-T1 | IMPL | Create `src/auth/mod.rs` exporting auth modules | `src/auth/mod.rs` | DONE |
| E2-T2 | IMPL | Create `src/auth/device_flow.rs` with `DeviceFlowAuth` struct implementing `initiate()` and `poll_for_token()` per Design §4.2 | `src/auth/device_flow.rs` | DONE |
| E2-T3 | IMPL | Implement `get_copilot_token()` in `DeviceFlowAuth` to exchange GitHub token for Copilot token | `src/auth/device_flow.rs` | DONE |
| E2-T4 | IMPL | Create `src/storage/mod.rs` and `src/storage/keyring.rs` with `TokenStorage` trait and `KeyringStorage` implementation | `src/storage/mod.rs`, `src/storage/keyring.rs` | DONE |
| E2-T5 | IMPL | Implement encrypted file fallback storage when keyring unavailable | `src/storage/file.rs` | DONE |
| E2-T6 | IMPL | Create `src/auth/token.rs` with `TokenManager` struct: `get_valid_token()`, `refresh_copilot_token()`, `start_auto_refresh()` per Design §4.4 | `src/auth/token.rs` | DONE |
| E2-T7 | IMPL | Wire `auth` CLI command to device flow, display user code and verification URL, store tokens on success | `src/main.rs`, `src/cli.rs` | DONE |
| E2-T8 | IMPL | Implement `logout` CLI command to clear stored credentials | `src/main.rs` | DONE |
| E2-T9 | TEST | Unit test: token expiry calculation correct; `is_valid()` returns false when expired | `tests/unit/token_tests.rs` | DONE |
| E2-T10 | TEST | Unit test: keyring storage round-trip (store → retrieve → delete) | `tests/unit/storage_tests.rs` | DONE |
| E2-T11 | TEST | Integration test: mock GitHub OAuth server; full device flow completes | `tests/integration/auth_tests.rs` | DONE |

**Acceptance Criteria:**
- [x] `copilot-adapter auth` prompts with verification URL and user code
- [x] After browser authorization, tokens stored in OS keyring
- [x] `copilot-adapter logout` removes stored tokens
- [x] Token refresh happens automatically before expiry

---

### Epic 3: API Implementation (Non-Streaming)

**Goal:** Implement the `/v1/chat/completions` endpoint for non-streaming requests and `/v1/models` endpoints.

**Prerequisites:** Epics 1, 2

**Tasks:**

| Task ID | Type | Description | Files | Status |
|---------|------|-------------|-------|--------|
| E3-T1 | IMPL | Create `src/copilot/mod.rs` exporting Copilot modules | `src/copilot/mod.rs` | TO DO |
| E3-T2 | IMPL | Create `src/copilot/types.rs` with `ChatCompletionRequest`, `ChatCompletionResponse`, `Message`, `Choice`, `Usage` structs matching OpenAI format | `src/copilot/types.rs` | TO DO |
| E3-T3 | IMPL | Create `src/copilot/client.rs` with `CopilotClient` struct; implement `send_chat_completion()` for non-streaming requests with required headers per Design §1.2 | `src/copilot/client.rs` | TO DO |
| E3-T4 | IMPL | Create `src/handlers/chat.rs` with `chat_completions` handler; validate request, get token, call Copilot client, return response | `src/handlers/chat.rs` | TO DO |
| E3-T5 | IMPL | Create `src/handlers/models.rs` with `list_models` and `get_model` handlers returning hardcoded Copilot model list per Design §1.3 | `src/handlers/models.rs` | TO DO |
| E3-T6 | IMPL | Register `/v1/chat/completions`, `/v1/models`, `/v1/models/:model` routes in `server.rs` | `src/server.rs` | TO DO |
| E3-T7 | IMPL | Create `AppState` struct with `Arc<TokenManager>` and `reqwest::Client`; wire into handlers | `src/server.rs` | TO DO |
| E3-T8 | TEST | Unit test: request/response types serialize correctly to/from JSON | `tests/unit/types_tests.rs` | TO DO |
| E3-T9 | TEST | Integration test: mock Copilot API; non-streaming chat completion round-trip | `tests/integration/chat_tests.rs` | TO DO |
| E3-T10 | TEST | Integration test: `/v1/models` returns expected model list | `tests/integration/models_tests.rs` | TO DO |

**Acceptance Criteria:**
- [ ] `POST /v1/chat/completions` with `stream: false` returns complete response
- [ ] Response format matches OpenAI specification (id, object, created, model, choices, usage)
- [ ] `GET /v1/models` returns list of available models
- [ ] `GET /v1/models/gpt-4` returns model details

---

### Epic 4: SSE Streaming Support

**Goal:** Implement Server-Sent Events streaming for real-time chat completion responses.

**Prerequisites:** Epic 3

**Tasks:**

| Task ID | Type | Description | Files | Status |
|---------|------|-------------|-------|--------|
| E4-T1 | IMPL | Add `ChatCompletionChunk`, `ChunkChoice`, `ChunkDelta` types to `copilot/types.rs` for streaming response format | `src/copilot/types.rs` | TO DO |
| E4-T2 | IMPL | Implement `stream_chat_completion()` in `CopilotClient` returning `impl Stream<Item = Result<ChatCompletionChunk>>` | `src/copilot/client.rs` | TO DO |
| E4-T3 | IMPL | Implement SSE parsing: buffer bytes, split on `\n\n`, extract `data:` lines, handle `[DONE]` marker | `src/copilot/client.rs` | TO DO |
| E4-T4 | IMPL | Update `chat_completions` handler to branch on `stream` field; return `Sse::new(stream)` for streaming | `src/handlers/chat.rs` | TO DO |
| E4-T5 | IMPL | Add `KeepAlive` to SSE response to prevent connection timeout | `src/handlers/chat.rs` | TO DO |
| E4-T6 | TEST | Unit test: SSE parser handles complete messages, partial buffers, and `[DONE]` marker | `tests/unit/sse_tests.rs` | TO DO |
| E4-T7 | TEST | Integration test: mock Copilot API with SSE; verify all chunks received in order | `tests/integration/streaming_tests.rs` | TO DO |
| E4-T8 | TEST | Integration test: concurrent streaming requests (5 simultaneous) complete successfully | `tests/integration/streaming_tests.rs` | TO DO |

**Acceptance Criteria:**
- [ ] `POST /v1/chat/completions` with `stream: true` returns SSE events
- [ ] Each chunk has `data:` prefix and valid JSON
- [ ] Stream ends with `data: [DONE]`
- [ ] Multiple concurrent streams handled correctly

---

### Epic 5: Background Daemon Operation

**Goal:** Implement cross-platform background process management with start/stop/status commands.

**Prerequisites:** Epic 1

**Tasks:**

| Task ID | Type | Description | Files | Status |
|---------|------|-------------|-------|--------|
| E5-T1 | IMPL | Create `src/daemon/mod.rs` with `get_pid_path()`, `is_running()`, `process_exists()` per Design §4.5 | `src/daemon/mod.rs` | TO DO |
| E5-T2 | IMPL | Create `src/daemon/unix.rs` with `daemonize()` using daemonize crate and `stop_daemon()` using SIGTERM | `src/daemon/unix.rs` | TO DO |
| E5-T3 | IMPL | Create `src/daemon/windows.rs` with background process spawn via `Command` and `stop_daemon()` via process termination | `src/daemon/windows.rs` | TO DO |
| E5-T4 | IMPL | Wire `start --daemon` flag to daemonize on Unix, background spawn on Windows | `src/main.rs` | TO DO |
| E5-T5 | IMPL | Implement `stop` command reading PID file and terminating process | `src/main.rs` | TO DO |
| E5-T6 | IMPL | Implement `status` command checking if process is running, displaying PID and port | `src/main.rs` | TO DO |
| E5-T7 | IMPL | Implement graceful shutdown on SIGTERM/SIGINT using tokio signal handlers | `src/server.rs` | TO DO |
| E5-T8 | IMPL | Add `--log-file` flag support; configure tracing-subscriber to write to file when specified | `src/main.rs` | TO DO |
| E5-T9 | TEST | Integration test (Unix): start daemon → status shows running → stop → status shows stopped | `tests/integration/daemon_tests.rs` | TO DO |
| E5-T10 | TEST | Integration test (Windows): equivalent daemon lifecycle test | `tests/integration/daemon_tests.rs` | TO DO |

**Acceptance Criteria:**
- [ ] `start --daemon` backgrounds the process and returns immediately
- [ ] `status` correctly reports running/stopped state
- [ ] `stop` terminates background process gracefully
- [ ] PID file created/cleaned up correctly
- [ ] Logs written to file when `--log-file` specified

---

### Epic 6: Error Handling and Logging

**Goal:** Implement comprehensive error handling with OpenAI-compatible error responses and configurable logging.

**Prerequisites:** Epics 1, 3

**Tasks:**

| Task ID | Type | Description | Files | Status |
|---------|------|-------------|-------|--------|
| E6-T1 | IMPL | Extend `AppError` enum with all error types from Design §8.1: `NotAuthenticated`, `TokenExpired`, `GitHubError`, `CopilotError`, `RateLimited`, `InvalidRequest`, `Internal` | `src/error.rs` | TO DO |
| E6-T2 | IMPL | Implement `IntoResponse` for `AppError` returning OpenAI-compatible JSON error format with appropriate HTTP status codes | `src/error.rs` | TO DO |
| E6-T3 | IMPL | Add rate limit handling: parse `Retry-After` header from Copilot API, return `RateLimited` error with seconds | `src/copilot/client.rs` | TO DO |
| E6-T4 | IMPL | Implement exponential backoff for transient errors (5xx, network timeouts) with 3 retries | `src/copilot/client.rs` | TO DO |
| E6-T5 | IMPL | Configure tracing-subscriber with env-filter; support `--log-level` flag and `RUST_LOG` env var | `src/main.rs` | TO DO |
| E6-T6 | IMPL | Add request tracing: log request ID, method, path, response status, duration | `src/server.rs` | TO DO |
| E6-T7 | IMPL | Add structured logging for auth events, token refresh, Copilot API calls | throughout | TO DO |
| E6-T8 | TEST | Unit test: each error type produces correct HTTP status and JSON format | `tests/unit/error_tests.rs` | TO DO |
| E6-T9 | TEST | Integration test: invalid request returns 400 with error JSON; auth failure returns 401 | `tests/integration/error_tests.rs` | TO DO |

**Acceptance Criteria:**
- [ ] All errors return OpenAI-compatible JSON format
- [ ] Rate limits surfaced with retry-after seconds
- [ ] Transient errors retried with exponential backoff
- [ ] Log level configurable via flag and env var
- [ ] Request logs include timing and status

---

### Epic 7: Testing and Documentation

**Goal:** Achieve comprehensive test coverage and complete documentation.

**Prerequisites:** Epics 1-6

**Tasks:**

| Task ID | Type | Description | Files | Status |
|---------|------|-------------|-------|--------|
| E7-T1 | TEST | Create mock GitHub OAuth server for integration tests using wiremock | `tests/common/mock_github.rs` | TO DO |
| E7-T2 | TEST | Create mock Copilot API server for integration tests using wiremock | `tests/common/mock_copilot.rs` | TO DO |
| E7-T3 | TEST | Write unit tests for token parsing, expiry calculation, validation | `tests/unit/token_tests.rs` | TO DO |
| E7-T4 | TEST | Write unit tests for all request/response type serialization | `tests/unit/types_tests.rs` | TO DO |
| E7-T5 | TEST | Write integration tests for full auth flow with mock servers | `tests/integration/auth_tests.rs` | TO DO |
| E7-T6 | TEST | Write integration tests for chat completions (streaming and non-streaming) | `tests/integration/chat_tests.rs` | TO DO |
| E7-T7 | TEST | Write integration tests for concurrent client handling (10+ simultaneous requests) | `tests/integration/concurrent_tests.rs` | TO DO |
| E7-T8 | DOC | Create README.md with installation, quick start, configuration, troubleshooting | `README.md` | TO DO |
| E7-T9 | DOC | Document manual E2E test procedures per Design §13 | `docs/e2e-testing.md` | TO DO |
| E7-T10 | TEST | Achieve >80% code coverage measured by cargo-tarpaulin | — | TO DO |

**Acceptance Criteria:**
- [ ] All unit tests pass
- [ ] All integration tests pass with mock servers
- [ ] Code coverage >80%
- [ ] README complete with all usage examples
- [ ] Manual E2E test procedures documented

---

## Verification Plan

After implementation, verify the adapter works per Design §13:

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

4. **Chat Completion Test (Non-Streaming)**
   ```bash
   curl -X POST http://127.0.0.1:8787/v1/chat/completions \
     -H "Content-Type: application/json" \
     -d '{"model": "gpt-4", "messages": [{"role": "user", "content": "Hello"}]}'
   # Should return complete response
   ```

5. **Chat Completion Test (Streaming)**
   ```bash
   curl -X POST http://127.0.0.1:8787/v1/chat/completions \
     -H "Content-Type: application/json" \
     -d '{"model": "gpt-4", "messages": [{"role": "user", "content": "Hello"}], "stream": true}'
   # Should receive SSE events ending with [DONE]
   ```

6. **Daemon Test**
   ```bash
   copilot-adapter start --daemon
   copilot-adapter status
   # Should show running
   copilot-adapter stop
   copilot-adapter status
   # Should show stopped
   ```

7. **Concurrent Clients Test**
   - Open 5 terminal windows
   - Send streaming requests simultaneously
   - Verify all receive complete responses

---

## References

| Document | Description |
|----------|-------------|
| [DESIGN.md](./DESIGN.md) | Full design document (source of truth) |
| [GitHub Device Flow](https://docs.github.com/en/apps/oauth-apps/building-oauth-apps/authorizing-oauth-apps#device-flow) | GitHub OAuth documentation |
| [OpenAI API Reference](https://platform.openai.com/docs/api-reference/chat) | OpenAI chat completions API spec |
| [axum Documentation](https://docs.rs/axum/latest/axum/) | HTTP server framework |
| [SSE Specification](https://html.spec.whatwg.org/multipage/server-sent-events.html) | Server-Sent Events standard |
