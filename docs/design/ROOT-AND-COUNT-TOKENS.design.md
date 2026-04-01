# Root Path and Token Counting — Design Document

**Status:** Proposed
**Date:** 2026-03-31
**Severity:** Low (Root), Medium (Token Counting)
**Related:** `MISSING-FEATURES.md`, `DYNAMIC-MODELS.design.md`

---

## Executive Summary

This document covers two related API compatibility features:

1. **Root Path Handler (`/`)** — Claude Code sends periodic `HEAD /` requests for health/availability probing. Currently returns 404, polluting logs and indicating a potentially broken adapter.

2. **Token Counting Endpoint (`/v1/messages/count_tokens`)** — Anthropic API endpoint for pre-flight token estimation. Used by Claude Code for context window management and cost estimation.

Both features improve Claude Code compatibility and user experience with minimal implementation effort.

---

## Part 1: Root Path Handler

### Problem Statement

Claude Code sends periodic `HEAD /` requests to verify the adapter is running:

```
2026-04-01T00:36:18.253773Z  INFO Request completed method=HEAD path=/ status=404
2026-04-01T00:36:32.075978Z  INFO Request completed method=HEAD path=/ status=404
2026-04-01T00:36:41.303656Z  INFO Request completed method=HEAD path=/ status=404
```

**Observed pattern:**
- Requests occur at session start
- Then periodically (~10-20 seconds apart)
- Uses HTTP `HEAD` method (status check, no body needed)
- Currently returns 404 (axum default for unmatched routes)

**Impact:**
- Log pollution with spurious 404 errors
- Potentially confusing for users monitoring logs
- Indicates incomplete API compatibility

### Research: LiteLLM Behavior

LiteLLM's root path behavior was analyzed for reference:

| Aspect | LiteLLM | copilot-adapter (proposed) |
|--------|---------|---------------------------|
| **Default Response** | `"LiteLLM: RUNNING"` (200 OK) | `{"status": "ok"}` (200 OK) |
| **HTTP Methods** | GET only (HEAD returns 405) | GET and HEAD |
| **Authentication** | Required | Not required |
| **Content-Type** | `application/json` | `application/json` |

**Key insight:** LiteLLM does NOT support HEAD requests on `/`, but Claude Code sends HEAD requests. Our implementation should support both GET and HEAD.

### Proposed Design

**Simple 200 OK response for both GET and HEAD:**

```rust
// In handlers/health.rs
pub async fn root() -> impl IntoResponse {
    Json(json!({"status": "ok"}))
}

// In server.rs - route registration
Router::new()
    .route("/", get(handlers::health::root).head(handlers::health::root))
```

**Response format:**
```http
HTTP/1.1 200 OK
Content-Type: application/json
Content-Length: 15

{"status":"ok"}
```

For HEAD requests, axum automatically strips the body while preserving headers.

### Design Decisions

| Decision | Rationale |
|----------|-----------|
| **Support both GET and HEAD** | Claude Code uses HEAD; browsers/curl use GET |
| **No authentication** | Health probes should work without auth |
| **JSON response** | Consistent with other endpoints |
| **Minimal body** | Fast, low overhead |
| **No redirect to /health** | Simpler; /health may have different semantics in future |

---

## Part 2: Token Counting Endpoint

### Problem Statement

The Anthropic API provides `POST /v1/messages/count_tokens` for pre-flight token estimation. This allows clients to:

1. **Context window management** — Know when approaching limits
2. **Cost estimation** — Calculate expected API costs
3. **Request validation** — Verify request is well-formed before sending

The copilot-adapter currently does not implement this endpoint.

### API Specification (Anthropic)

**Endpoint:** `POST /v1/messages/count_tokens`

**Request body (same structure as `/v1/messages`, minus execution params):**

```typescript
interface CountTokensRequest {
  model: string;                      // Required - model identifier
  messages: AnthropicMessage[];       // Required - conversation messages
  system?: string | ContentBlock[];   // Optional - system prompt
  tools?: ToolDefinition[];           // Optional - tool definitions
}
```

**Response:**

```typescript
interface CountTokensResponse {
  input_tokens: number;  // Total input tokens
}
```

**Example:**

```bash
curl -X POST http://localhost:6767/v1/messages/count_tokens \
  -H "Content-Type: application/json" \
  -d '{
    "model": "claude-sonnet-4-20250514",
    "messages": [{"role": "user", "content": "Hello, Claude!"}],
    "system": "You are a helpful assistant."
  }'
```

Response:
```json
{"input_tokens": 18}
```

### Research: LiteLLM Implementation

LiteLLM implements `/v1/messages/count_tokens` in `litellm/proxy/anthropic_endpoints/endpoints.py`:

```python
@router.post("/v1/messages/count_tokens", ...)
async def count_tokens(request: Request):
    # 1. Parse request body (same as /v1/messages)
    # 2. Create TokenCountRequest
    # 3. Call internal token_counter()
    #    - Tries provider-specific API (OpenAI, Anthropic) if available
    #    - Falls back to local tiktoken
    # 4. Return {"input_tokens": count}
```

**Key insight:** LiteLLM uses a multi-tier approach:
1. Try provider's native count_tokens API (if available)
2. Fall back to local tiktoken-based counting

Since GitHub Copilot has no token counting API, we must use local counting.

### Implementation Options

| Option | Accuracy | Binary Size | Effort | Notes |
|--------|----------|-------------|--------|-------|
| **A: tiktoken-rs** | ~95% | +1-2 MB | Medium | Uses `cl100k_base` encoding |
| **B: Character estimate** | ~60% | +0 | Low | ~4 chars/token heuristic |
| **C: Return error** | N/A | +0 | Very Low | Document as unsupported |

**Recommendation: Option A (tiktoken-rs)**

Rationale:
- Accurate enough for practical context window management
- Same tokenizer used by Claude models (`cl100k_base`)
- No external API dependencies
- LiteLLM uses the same approach as fallback

### Proposed Design

#### New Types

```rust
// src/anthropic/types.rs

/// Request body for /v1/messages/count_tokens
#[derive(Debug, Clone, Deserialize)]
pub struct CountTokensRequest {
    pub model: String,
    pub messages: Vec<AnthropicMessage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub system: Option<SystemInput>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tools: Option<Vec<ToolDefinition>>,
}

/// Response body for /v1/messages/count_tokens
#[derive(Debug, Clone, Serialize)]
pub struct CountTokensResponse {
    pub input_tokens: u32,
}
```

#### Token Counting Module

```rust
// src/token_counter.rs

use tiktoken_rs::cl100k_base;

/// Count tokens for a count_tokens request using tiktoken.
pub fn count_tokens(request: &CountTokensRequest) -> Result<u32, TokenCountError> {
    let bpe = cl100k_base().map_err(|e| TokenCountError::EncoderInit(e.to_string()))?;
    let mut total: usize = 0;

    // 1. System prompt
    if let Some(system) = &request.system {
        total += bpe.encode_with_special_tokens(&system.to_text()).len();
    }

    // 2. Messages (with per-message overhead for role/formatting)
    for msg in &request.messages {
        total += 4; // Role overhead (~4 tokens per message)
        total += count_content_tokens(&bpe, &msg.content);
    }

    // 3. Tool definitions
    if let Some(tools) = &request.tools {
        for tool in tools {
            let tool_json = serde_json::to_string(tool)
                .map_err(|e| TokenCountError::Serialization(e.to_string()))?;
            total += bpe.encode_with_special_tokens(&tool_json).len();
        }
    }

    Ok(total as u32)
}

fn count_content_tokens(bpe: &CoreBPE, content: &ContentBlockInput) -> usize {
    match content {
        ContentBlockInput::Text(s) => bpe.encode_with_special_tokens(s).len(),
        ContentBlockInput::Blocks(blocks) => {
            blocks.iter().map(|block| {
                match block {
                    ContentBlock::Text { text, .. } => {
                        bpe.encode_with_special_tokens(text).len()
                    }
                    ContentBlock::Image { .. } => {
                        // Images: ~65 base tokens + resolution overhead
                        // Using conservative estimate
                        85
                    }
                    ContentBlock::Document { .. } => {
                        // Documents not fully supported; estimate
                        100
                    }
                    ContentBlock::ToolUse { input, .. } => {
                        let json = serde_json::to_string(input).unwrap_or_default();
                        bpe.encode_with_special_tokens(&json).len() + 10
                    }
                    ContentBlock::ToolResult { content, .. } => {
                        match content {
                            ToolResultContent::Text(s) => {
                                bpe.encode_with_special_tokens(s).len()
                            }
                            ToolResultContent::Blocks(inner) => {
                                // Recursive but limited depth
                                inner.iter().filter_map(|b| match b {
                                    ContentBlock::Text { text, .. } => {
                                        Some(bpe.encode_with_special_tokens(text).len())
                                    }
                                    _ => None
                                }).sum()
                            }
                        }
                    }
                }
            }).sum()
        }
    }
}
```

#### Handler

```rust
// src/handlers/count_tokens.rs

use axum::{extract::State, Json};
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
    let input_tokens = token_counter::count_tokens(&request)
        .map_err(|e| ApiError::Internal(format!("Token counting failed: {e}")))?;

    Ok(Json(CountTokensResponse { input_tokens }))
}
```

#### Route Registration

```rust
// src/server.rs

pub fn build_router(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/", get(handlers::health::root).head(handlers::health::root))
        .route("/health", get(handlers::health::health))
        .route("/v1/messages", post(handlers::messages::messages))
        .route("/v1/messages/count_tokens", post(handlers::count_tokens::count_tokens))
        .route("/v1/models", get(handlers::models::list_models))
        .route("/v1/models/:model", get(handlers::models::get_model))
        .with_state(state)
        // ... middleware
}
```

### Design Decisions

| Decision | Rationale |
|----------|-----------|
| **Use tiktoken-rs** | Accurate, same tokenizer as Claude models |
| **cl100k_base encoding** | Standard for GPT-4/Claude-class models |
| **No caching** | Requests vary; counting is fast (~1ms) |
| **Conservative image estimates** | 85 tokens covers most cases |
| **No authentication** | Matches /v1/messages behavior |
| **Lazy encoder init** | First call may be slow; subsequent calls fast |

### Token Count Accuracy

| Content Type | Accuracy | Notes |
|--------------|----------|-------|
| Plain text | ~99% | Direct tokenization |
| System prompt | ~99% | Direct tokenization |
| Tool definitions | ~95% | JSON serialization adds overhead |
| Images | ~80% | Fixed estimate (actual varies by resolution) |
| Documents | ~70% | Not fully supported by adapter |
| Messages with tools | ~95% | Combined accuracy |

---

## File Changes Summary

| File | Change |
|------|--------|
| `Cargo.toml` | Add `tiktoken-rs` dependency |
| `src/handlers/mod.rs` | Export `count_tokens` module |
| `src/handlers/health.rs` | Add `root()` handler |
| `src/handlers/count_tokens.rs` | **New file** — count_tokens handler |
| `src/token_counter.rs` | **New file** — token counting logic |
| `src/anthropic/types.rs` | Add `CountTokensRequest`, `CountTokensResponse` |
| `src/server.rs` | Register `/` and `/v1/messages/count_tokens` routes |
| `src/error.rs` | Add `TokenCountError` variant (optional) |

---

## Testing Strategy

### Root Path Tests

1. **GET /** returns 200 with `{"status": "ok"}`
2. **HEAD /** returns 200 with empty body
3. Response includes correct `Content-Type: application/json`

### Token Counting Tests

1. **Simple text message** — verify count is reasonable
2. **System prompt** — included in count
3. **Multiple messages** — counts all messages
4. **Tool definitions** — included in count
5. **Empty messages array** — returns 0 or error
6. **Missing model** — returns 400 error
7. **Image blocks** — uses estimate
8. **Large request** — performance acceptable (<10ms)

### Integration Tests

1. Start adapter, send `HEAD /` — verify 200
2. Start adapter, send count_tokens request — verify response format
3. Compare counts with actual /v1/messages usage (rough validation)

---

## Risks and Mitigations

| Risk | Likelihood | Impact | Mitigation |
|------|------------|--------|------------|
| tiktoken-rs adds binary size | Certain | Low | ~1-2MB acceptable; consider feature flag |
| Token counts differ from actual | Medium | Low | Document as estimates; sufficient for context management |
| Encoder init slow on first call | Medium | Low | Lazy init; subsequent calls fast |
| Claude Code doesn't use count_tokens | Medium | Low | No harm; improves API compatibility |

---

## Open Questions

| # | Question | Status |
|---|----------|--------|
| 1 | Does Claude Code actually call count_tokens? | Unknown — no evidence in logs |
| 2 | Should we cache the tiktoken encoder? | Deferred — measure performance first |
| 3 | Should count_tokens require auth header? | No — matches /v1/messages behavior |
| 4 | Should we add a feature flag for tiktoken? | Deferred — binary size impact acceptable |

---

## References

- [Anthropic Messages API](https://docs.anthropic.com/en/api/messages)
- [Anthropic Token Counting](https://docs.anthropic.com/en/api/counting-tokens)
- [tiktoken-rs crate](https://crates.io/crates/tiktoken-rs)
- [LiteLLM count_tokens implementation](https://github.com/BerriAI/litellm)
- `docs/design/MISSING-FEATURES.md` — Section 2: Token Counting
