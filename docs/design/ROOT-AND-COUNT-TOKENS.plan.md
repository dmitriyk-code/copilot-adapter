# Root Path and Token Counting — Implementation Plan

**Status:** In Progress
**Date:** 2026-03-31
**Based on:** [ROOT-AND-COUNT-TOKENS.design.md](./ROOT-AND-COUNT-TOKENS.design.md)
**Prerequisite:** Core adapter implementation — COMPLETE

---

## Executive Summary

This plan implements two API compatibility features:

1. **Root Path Handler (`/`)** — Simple 200 OK response for health probes (eliminates 404 log noise)
2. **Token Counting Endpoint (`/v1/messages/count_tokens`)** — Pre-flight token estimation using tiktoken-rs

Both features improve Claude Code compatibility with minimal effort. The root path handler is a quick win (~30 minutes); token counting is a medium-effort feature (~4-6 hours).

---

## Background

### Current State

**Root path:**
- Claude Code sends `HEAD /` requests for health probing
- Returns 404 (unmatched route)
- Pollutes logs with spurious errors

**Token counting:**
- No `/v1/messages/count_tokens` endpoint exists
- Documented in `MISSING-FEATURES.md` as Medium Priority
- LiteLLM implements this using tiktoken fallback

### Target State

**Root path:**
- `GET /` and `HEAD /` return 200 OK with `{"status": "ok"}`
- No authentication required
- Clean logs without 404 noise

**Token counting:**
- `POST /v1/messages/count_tokens` accepts same format as `/v1/messages`
- Returns `{"input_tokens": N}` using tiktoken-rs
- Accurate enough for context window management

---

## Goals and Non-Goals

### Goals

| ID | Goal | Success Criteria |
|----|------|------------------|
| G1 | Root path returns 200 OK | `GET /` and `HEAD /` return 200 |
| G2 | No 404 log noise | Logs show 200 for root path requests |
| G3 | Token counting endpoint exists | `POST /v1/messages/count_tokens` responds |
| G4 | Token counts are accurate | Within ~5% of actual usage for text |
| G5 | Response format matches Anthropic | Returns `{"input_tokens": N}` |
| G6 | Performance acceptable | Token counting <10ms for typical requests |

### Non-Goals

| ID | Non-Goal | Rationale |
|----|----------|-----------|
| NG1 | Exact token count accuracy | Estimates sufficient for context management |
| NG2 | Provider-specific counting | No Copilot API for this; local only |
| NG3 | Output token estimation | Anthropic API doesn't return this either |
| NG4 | Caching token counts | Requests vary; counting is fast |

---

## Requirements

### Functional Requirements

| ID | Requirement | Source |
|----|-------------|--------|
| FR1 | `GET /` returns 200 OK with JSON body | Design: Part 1 |
| FR2 | `HEAD /` returns 200 OK with empty body | Design: Part 1 |
| FR3 | Root path requires no authentication | Design: Part 1 |
| FR4 | `POST /v1/messages/count_tokens` accepts CountTokensRequest | Design: Part 2 |
| FR5 | Token counting returns `{"input_tokens": N}` | Design: Part 2 |
| FR6 | Token counting includes system prompt | Design: Part 2 |
| FR7 | Token counting includes tool definitions | Design: Part 2 |
| FR8 | Missing `model` returns 400 error | Design: Part 2 |
| FR9 | Missing `messages` returns 400 error | Design: Part 2 |

### Non-Functional Requirements

| ID | Requirement | Target |
|----|-------------|--------|
| NFR1 | Root path response time | <1ms |
| NFR2 | Token counting response time | <10ms for typical requests |
| NFR3 | Binary size increase (tiktoken) | <2MB acceptable |
| NFR4 | Token count accuracy (text) | >95% |
| NFR5 | Token count accuracy (images) | >70% (estimates) |

---

## Dependencies

### New Dependencies

| Crate | Version | Purpose | Size Impact |
|-------|---------|---------|-------------|
| `tiktoken-rs` | `0.5` | BPE tokenization | ~1-2MB (vocabulary data) |

### Existing Dependencies Used

- `axum` — HTTP routing and handlers
- `serde` / `serde_json` — Request/response serialization
- `tokio` — Async runtime

---

## Implementation Plan

### Epic 1: Root Path Handler — ✅ COMPLETE

**Goal:** Add `/` route returning 200 OK for GET and HEAD requests.

**Prerequisites:** None

**Estimated Effort:** 30 minutes

**Tasks:**

| Task ID | Type | Description | Files | Est. |
|---------|------|-------------|-------|------|
| E1-T1 | IMPL | Add `root()` handler function in `handlers/health.rs` | `src/handlers/health.rs` | 5m |
| E1-T2 | IMPL | Register `/` route for GET and HEAD in `build_router()` | `src/server.rs` | 5m |
| E1-T3 | TEST | Unit test: GET / returns 200 with JSON body | `tests/unit/` | 10m |
| E1-T4 | TEST | Unit test: HEAD / returns 200 with empty body | `tests/unit/` | 5m |
| E1-T5 | TEST | Manual test: verify log shows 200 not 404 | Manual | 5m |

**Acceptance Criteria:**
- [x] `curl http://localhost:6767/` returns `{"status":"ok"}`
- [x] `curl -I http://localhost:6767/` returns 200 OK
- [x] Logs show `status=200` for root path requests

**Implementation Details:**

```rust
// src/handlers/health.rs

use axum::Json;
use serde_json::json;

/// Handler for GET / and HEAD /
/// Returns simple status for health probes.
pub async fn root() -> Json<serde_json::Value> {
    Json(json!({"status": "ok"}))
}
```

```rust
// src/server.rs (in build_router)

Router::new()
    .route("/", get(handlers::health::root).head(handlers::health::root))
    // ... existing routes
```

---

### Epic 2: Token Counting Types

**Goal:** Define request and response types for token counting.

**Prerequisites:** None (can start in parallel with Epic 1)

**Estimated Effort:** 30 minutes

**Tasks:**

| Task ID | Type | Description | Files | Est. |
|---------|------|-------------|-------|------|
| E2-T1 | IMPL | Add `CountTokensRequest` struct | `src/anthropic/types.rs` | 10m |
| E2-T2 | IMPL | Add `CountTokensResponse` struct | `src/anthropic/types.rs` | 5m |
| E2-T3 | TEST | Unit test: deserialize valid request | `tests/unit/` | 10m |
| E2-T4 | TEST | Unit test: deserialize request with optional fields | `tests/unit/` | 5m |

**Acceptance Criteria:**
- [ ] `CountTokensRequest` deserializes from JSON
- [ ] All fields (model, messages, system, tools) handled correctly
- [ ] `CountTokensResponse` serializes to `{"input_tokens": N}`

**Implementation Details:**

```rust
// src/anthropic/types.rs

/// Request body for POST /v1/messages/count_tokens
#[derive(Debug, Clone, Deserialize)]
pub struct CountTokensRequest {
    pub model: String,
    pub messages: Vec<AnthropicMessage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub system: Option<SystemInput>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tools: Option<Vec<ToolDefinition>>,
}

/// Response body for POST /v1/messages/count_tokens
#[derive(Debug, Clone, Serialize)]
pub struct CountTokensResponse {
    pub input_tokens: u32,
}
```

---

### Epic 3: Token Counter Module

**Goal:** Implement token counting logic using tiktoken-rs.

**Prerequisites:** Epic 2 (types)

**Estimated Effort:** 2-3 hours

**Tasks:**

| Task ID | Type | Description | Files | Est. |
|---------|------|-------------|-------|------|
| E3-T1 | IMPL | Add `tiktoken-rs` to Cargo.toml | `Cargo.toml` | 5m |
| E3-T2 | IMPL | Create `src/token_counter.rs` module file | `src/token_counter.rs` | 5m |
| E3-T3 | IMPL | Add `TokenCountError` enum | `src/token_counter.rs` | 10m |
| E3-T4 | IMPL | Implement `count_tokens()` main function | `src/token_counter.rs` | 30m |
| E3-T5 | IMPL | Implement `count_content_tokens()` helper | `src/token_counter.rs` | 30m |
| E3-T6 | IMPL | Handle text content blocks | `src/token_counter.rs` | 10m |
| E3-T7 | IMPL | Handle image blocks (fixed estimate) | `src/token_counter.rs` | 10m |
| E3-T8 | IMPL | Handle tool_use and tool_result blocks | `src/token_counter.rs` | 15m |
| E3-T9 | IMPL | Export module from `lib.rs` | `src/lib.rs` | 5m |
| E3-T10 | TEST | Unit test: count simple text message | `tests/unit/` | 10m |
| E3-T11 | TEST | Unit test: count with system prompt | `tests/unit/` | 10m |
| E3-T12 | TEST | Unit test: count with tools | `tests/unit/` | 10m |
| E3-T13 | TEST | Unit test: count with multiple messages | `tests/unit/` | 10m |
| E3-T14 | TEST | Unit test: image block uses estimate | `tests/unit/` | 5m |

**Acceptance Criteria:**
- [ ] `count_tokens()` returns reasonable counts for text
- [ ] System prompts included in count
- [ ] Tool definitions included in count
- [ ] Image blocks use fixed estimate (~85 tokens)
- [ ] All unit tests pass

**Implementation Details:**

```rust
// src/token_counter.rs

use tiktoken_rs::cl100k_base;
use crate::anthropic::types::{
    ContentBlock, ContentBlockInput, CountTokensRequest,
    SystemInput, ToolResultContent,
};

/// Errors that can occur during token counting.
#[derive(Debug, thiserror::Error)]
pub enum TokenCountError {
    #[error("Failed to initialize tokenizer: {0}")]
    EncoderInit(String),
    #[error("Failed to serialize: {0}")]
    Serialization(String),
}

/// Count tokens for a CountTokensRequest using tiktoken cl100k_base.
pub fn count_tokens(request: &CountTokensRequest) -> Result<u32, TokenCountError> {
    let bpe = cl100k_base()
        .map_err(|e| TokenCountError::EncoderInit(e.to_string()))?;

    let mut total: usize = 0;

    // System prompt
    if let Some(system) = &request.system {
        total += bpe.encode_with_special_tokens(&system.to_text()).len();
    }

    // Messages
    for msg in &request.messages {
        total += 4; // Per-message overhead
        total += count_content_tokens(&bpe, &msg.content)?;
    }

    // Tool definitions
    if let Some(tools) = &request.tools {
        for tool in tools {
            let json = serde_json::to_string(tool)
                .map_err(|e| TokenCountError::Serialization(e.to_string()))?;
            total += bpe.encode_with_special_tokens(&json).len();
        }
    }

    Ok(total as u32)
}
```

---

### Epic 4: Count Tokens Handler

**Goal:** Implement HTTP handler and route for `/v1/messages/count_tokens`.

**Prerequisites:** Epics 2 and 3

**Estimated Effort:** 1 hour

**Tasks:**

| Task ID | Type | Description | Files | Est. |
|---------|------|-------------|-------|------|
| E4-T1 | IMPL | Create `src/handlers/count_tokens.rs` | `src/handlers/count_tokens.rs` | 15m |
| E4-T2 | IMPL | Implement `count_tokens()` handler | `src/handlers/count_tokens.rs` | 15m |
| E4-T3 | IMPL | Add error handling (400 for bad requests) | `src/handlers/count_tokens.rs` | 10m |
| E4-T4 | IMPL | Export module from `handlers/mod.rs` | `src/handlers/mod.rs` | 5m |
| E4-T5 | IMPL | Register route in `build_router()` | `src/server.rs` | 5m |
| E4-T6 | TEST | Integration test: valid request returns count | `tests/integration/` | 10m |
| E4-T7 | TEST | Integration test: missing model returns 400 | `tests/integration/` | 5m |
| E4-T8 | TEST | Integration test: empty messages returns count | `tests/integration/` | 5m |

**Acceptance Criteria:**
- [ ] `POST /v1/messages/count_tokens` responds with 200
- [ ] Response format is `{"input_tokens": N}`
- [ ] Invalid requests return 400 with error message
- [ ] All integration tests pass

**Implementation Details:**

```rust
// src/handlers/count_tokens.rs

use axum::{extract::State, http::StatusCode, Json};
use std::sync::Arc;

use crate::anthropic::types::{CountTokensRequest, CountTokensResponse};
use crate::error::ApiError;
use crate::server::AppState;
use crate::token_counter;

/// Handler for POST /v1/messages/count_tokens
pub async fn count_tokens(
    State(_state): State<Arc<AppState>>,
    Json(request): Json<CountTokensRequest>,
) -> Result<Json<CountTokensResponse>, ApiError> {
    // Validate required fields
    if request.messages.is_empty() {
        return Err(ApiError::BadRequest("messages array is required".into()));
    }

    let input_tokens = token_counter::count_tokens(&request)
        .map_err(|e| ApiError::Internal(format!("Token counting failed: {e}")))?;

    Ok(Json(CountTokensResponse { input_tokens }))
}
```

```rust
// src/server.rs (in build_router)

.route(
    "/v1/messages/count_tokens",
    axum::routing::post(handlers::count_tokens::count_tokens),
)
```

---

### Epic 5: Testing and Documentation

**Goal:** Comprehensive testing and documentation updates.

**Prerequisites:** Epics 1-4

**Estimated Effort:** 1-2 hours

**Tasks:**

| Task ID | Type | Description | Files | Est. |
|---------|------|-------------|-------|------|
| E5-T1 | TEST | E2E test: root path with curl | Manual | 5m |
| E5-T2 | TEST | E2E test: count_tokens with curl | Manual | 10m |
| E5-T3 | TEST | E2E test: count_tokens with Claude Code (if possible) | Manual | 15m |
| E5-T4 | TEST | Performance test: count_tokens <10ms | Manual | 10m |
| E5-T5 | DOC | Update README with count_tokens endpoint | `README.md` | 15m |
| E5-T6 | DOC | Update CLAUDE.md with feature notes | `CLAUDE.md` | 10m |
| E5-T7 | DOC | Add to docs/e2e-testing.md | `docs/e2e-testing.md` | 15m |
| E5-T8 | DOC | Update MISSING-FEATURES.md status | `docs/design/MISSING-FEATURES.md` | 5m |

**Acceptance Criteria:**
- [ ] All E2E tests pass
- [ ] Performance meets NFR2 (<10ms)
- [ ] README documents new endpoints
- [ ] CLAUDE.md updated
- [ ] E2E testing procedures documented

---

## Verification Plan

### Root Path Verification

```bash
# Test GET /
curl -v http://localhost:6767/
# Expected: 200 OK, {"status":"ok"}

# Test HEAD /
curl -I http://localhost:6767/
# Expected: 200 OK, empty body

# Verify logs
copilot-adapter start --log-level debug
# Send HEAD / request
# Check logs show status=200 not status=404
```

### Token Counting Verification

```bash
# Simple message
curl -X POST http://localhost:6767/v1/messages/count_tokens \
  -H "Content-Type: application/json" \
  -d '{
    "model": "claude-sonnet-4-20250514",
    "messages": [{"role": "user", "content": "Hello!"}]
  }'
# Expected: {"input_tokens": ~5-10}

# With system prompt
curl -X POST http://localhost:6767/v1/messages/count_tokens \
  -H "Content-Type: application/json" \
  -d '{
    "model": "claude-sonnet-4-20250514",
    "messages": [{"role": "user", "content": "Hello!"}],
    "system": "You are a helpful assistant."
  }'
# Expected: {"input_tokens": ~15-20}

# With tools
curl -X POST http://localhost:6767/v1/messages/count_tokens \
  -H "Content-Type: application/json" \
  -d '{
    "model": "claude-sonnet-4-20250514",
    "messages": [{"role": "user", "content": "Search for foo"}],
    "tools": [{"name": "Grep", "description": "Search files", "input_schema": {"type": "object", "properties": {"pattern": {"type": "string"}}}}]
  }'
# Expected: {"input_tokens": ~50-80}

# Error case: missing messages
curl -X POST http://localhost:6767/v1/messages/count_tokens \
  -H "Content-Type: application/json" \
  -d '{"model": "claude-sonnet-4-20250514"}'
# Expected: 400 Bad Request

# Performance test
time curl -X POST http://localhost:6767/v1/messages/count_tokens \
  -H "Content-Type: application/json" \
  -d '{
    "model": "claude-sonnet-4-20250514",
    "messages": [{"role": "user", "content": "'"$(printf 'x%.0s' {1..10000})"'"}]
  }'
# Expected: <10ms total
```

---

## File Changes Summary

### New Files

| File | Purpose |
|------|---------|
| `src/token_counter.rs` | Token counting logic with tiktoken |
| `src/handlers/count_tokens.rs` | HTTP handler for count_tokens endpoint |

### Modified Files

| File | Changes |
|------|---------|
| `Cargo.toml` | Add `tiktoken-rs` dependency |
| `src/lib.rs` | Export `token_counter` module |
| `src/handlers/mod.rs` | Export `count_tokens` module |
| `src/handlers/health.rs` | Add `root()` handler |
| `src/server.rs` | Register `/` and `/v1/messages/count_tokens` routes |
| `src/anthropic/types.rs` | Add `CountTokensRequest`, `CountTokensResponse` |
| `README.md` | Document new endpoints |
| `CLAUDE.md` | Add feature notes |
| `docs/e2e-testing.md` | Add test procedures |
| `docs/design/MISSING-FEATURES.md` | Update status |

---

## Risks and Mitigations

| Risk | Likelihood | Impact | Mitigation |
|------|------------|--------|------------|
| tiktoken-rs compilation issues | Low | Medium | Test on CI; fallback to character estimate |
| Binary size increase unacceptable | Low | Low | Feature flag if needed; ~2MB is acceptable |
| Token counts significantly inaccurate | Low | Low | Document as estimates; test against actual usage |
| Claude Code doesn't use count_tokens | Medium | Low | Still improves API compatibility |

---

## Rollout Plan

### Phase 1: Root Path Handler (Day 1)
- [x] Implement Epic 1 (root path)
- [x] Verify with curl
- [x] Verify logs show 200

### Phase 2: Token Counting Types (Day 1)
- [ ] Implement Epic 2 (types)
- [ ] Unit tests pass

### Phase 3: Token Counter Module (Day 1-2)
- [ ] Add tiktoken-rs dependency
- [ ] Implement Epic 3 (counting logic)
- [ ] Unit tests pass

### Phase 4: Handler and Route (Day 2)
- [ ] Implement Epic 4 (handler)
- [ ] Integration tests pass
- [ ] E2E verification

### Phase 5: Documentation (Day 2)
- [ ] Complete Epic 5 (testing/docs)
- [ ] Update README, CLAUDE.md
- [ ] Update design doc status to "Implemented"

---

## Open Questions

| # | Question | Resolution |
|---|----------|------------|
| 1 | Should we add `thiserror` dependency for TokenCountError? | Use existing error pattern or add if not present |
| 2 | Should count_tokens require API key header? | No — matches /v1/messages which doesn't require it |
| 3 | Should we validate model name exists? | No — just count tokens; model validation at /v1/messages |
| 4 | Cache tiktoken encoder in AppState? | Start without; add if performance is an issue |

---

## References

| Document | Description |
|----------|-------------|
| [ROOT-AND-COUNT-TOKENS.design.md](./ROOT-AND-COUNT-TOKENS.design.md) | Design document |
| [MISSING-FEATURES.md](./MISSING-FEATURES.md) | Feature gap analysis |
| [tiktoken-rs docs](https://docs.rs/tiktoken-rs) | Tokenizer library |
| [Anthropic Token Counting](https://docs.anthropic.com/en/api/counting-tokens) | API reference |
