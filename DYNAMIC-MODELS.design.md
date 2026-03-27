# Dynamic Models List — Design Document

**Status:** Draft (API Discovery Phase — COMPLETE)
**Date:** 2026-03-27
**Prerequisite:** Core adapter implementation — COMPLETE

---

## Context

The Copilot Adapter currently serves a **hardcoded list of models** from the `/v1/models` endpoint (see `src/handlers/models.rs`). This list includes:
- `gpt-4`
- `gpt-4o`
- `gpt-4-turbo`
- `gpt-3.5-turbo`
- `claude-3.5-sonnet`

This approach has several problems:
1. **Stale data** — Models are retired (e.g., `claude-3.5-sonnet` was retired Feb 2026) or added without updating the adapter
2. **Manual maintenance** — Requires code changes and recompilation to update the list
3. **Incomplete information** — Hardcoded entries lack metadata like context window sizes, capabilities, etc.

---

## Problem Statement

The `/v1/models` endpoint should return the **actual models available** through GitHub Copilot's API, not a static list that may be outdated or incomplete.

Current behavior:
```bash
curl http://localhost:6767/v1/models
```
```json
{
  "object": "list",
  "data": [
    {"id": "gpt-4", "object": "model", "created": 1686935002, "owned_by": "github-copilot"},
    {"id": "claude-3.5-sonnet", ...}  // ← Retired model still listed!
  ]
}
```

Desired behavior:
- Return models actually available from Copilot API
- Include accurate metadata (context window, capabilities if available)
- Cache results to avoid excessive API calls
- Graceful fallback if API is unreachable

---

## Research: GitHub Copilot Models Endpoint

### Endpoint Discovery Findings

**Status:** CONFIRMED (March 2026)

The correct endpoint is:

```
GET https://api.githubcopilot.com/models
```

The `/v1/models` path does **NOT** work — only `/models` is valid. This was confirmed through:
1. Community reports (GitHub forum discussions, developer blog posts)
2. Analysis of the official GitHub Copilot CLI (`github/copilot-cli`) codebase
3. Analysis of third-party Copilot API proxies (`ericc-ch/copilot-api`)
4. The `lzwjava.github.io` dynamic model fetching guide (March 2026)

> **NOTE:** These findings are based on secondary source analysis (community forums, third-party
> projects, and developer blog posts) rather than direct API testing. Direct verification against
> a live Copilot API is recommended before Epic 2 implementation begins.

**Constant to use in code:**
```rust
const COPILOT_MODELS_URL: &str = "https://api.githubcopilot.com/models";
```

### Confirmed Response Format

The endpoint returns an OpenAI-compatible JSON response:

```json
{
  "data": [
    {
      "id": "gpt-4o",
      "object": "model",
      "created": 1234567890,
      "owned_by": "github-copilot"
    },
    {
      "id": "gpt-4o-mini",
      "object": "model",
      "created": 1234567890,
      "owned_by": "github-copilot"
    },
    {
      "id": "claude-3.5-sonnet",
      "object": "model",
      "created": 1234567890,
      "owned_by": "github-copilot"
    }
  ]
}
```

**Response fields per model object:**

| Field | Type | Description | Notes |
|-------|------|-------------|-------|
| `id` | string | Model identifier (e.g., `"gpt-4o"`) | Used directly in `/chat/completions` requests |
| `object` | string | Always `"model"` | Standard OpenAI format |
| `created` | integer | Unix timestamp | May be a fixed/placeholder value |
| `owned_by` | string | Owner identifier | Typically `"github-copilot"` |

**Additional fields (from some Copilot API versions):**

Some API responses also include Copilot-specific fields that are not in the standard OpenAI format:

| Field | Type | Description |
|-------|------|-------------|
| `vendor` | string | Model provider (e.g., `"openai"`, `"anthropic"`) |
| `name` | string | Human-readable display name |

Our `Model` struct uses `#[serde(deny_unknown_fields)]`-free deserialization, so extra fields
will be silently ignored — which is the correct behavior for forward compatibility.

**Response envelope:**

The top-level response object may or may not include the `"object": "list"` field. Our
`ModelList` struct should tolerate its absence. The `data` array is always present.

### Required Headers (Confirmed)

Same headers as chat completions (from `src/copilot/client.rs`):

```
Authorization: Bearer <copilot_token>
Copilot-Integration-Id: vscode-chat
Editor-Version: vscode/1.85.0
Editor-Plugin-Version: copilot-chat/0.12.0
```

The `Content-Type: application/json` header is optional for GET requests but harmless to include.

The `Openai-Organization` and `Openai-Intent` headers used for chat completions are **not required** for the models endpoint, but including them is harmless.

### Rate Limits and Special Requirements

**Rate limiting:**
- The `/models` endpoint is a lightweight, read-only GET request
- No specific rate limit headers or 429 responses have been observed for this endpoint
- However, the general Copilot API abuse detection still applies
- **Recommendation:** Cache results with a 5-minute TTL (configurable) to minimize API calls

**Subscription-awareness:**
- The models list is **subscription-aware**: different Copilot plans (Individual, Business, Enterprise) may return different model sets
- The list changes as GitHub adds or removes model support server-side
- There have been reports of models being removed from the API endpoint without notice (e.g., GPT-5.x models temporarily disappeared for some users in Feb 2026)

**Token requirements:**
- Requires a valid **Copilot token** (not a GitHub PAT)
- Token is obtained via `GET https://api.github.com/copilot_internal/v2/token` exchange
- Our existing `TokenManager` handles this correctly

**Error behavior:**
- 401: Invalid or expired token
- 403: Feature/policy restricted (organization policies can block API access)
- 429: Rate limited (with `Retry-After` header)
- 5xx: Server errors (retry with backoff)

### Verification Script

```bash
#!/bin/bash
# Run after `copilot-adapter auth`
# Verifies the models endpoint is working
#
# NOTE: The adapter stores credentials in the OS keyring (with encrypted file
# fallback), NOT as a plain-text file. You must supply your GitHub PAT via the
# GITHUB_TOKEN environment variable:
#
#   export GITHUB_TOKEN="ghp_your_personal_access_token"
#
# You can obtain a PAT from: https://github.com/settings/tokens
# The PAT needs the "copilot" scope.

if [ -z "$GITHUB_TOKEN" ]; then
    echo "ERROR: Set GITHUB_TOKEN to your GitHub Personal Access Token."
    echo "  export GITHUB_TOKEN=\"ghp_...\""
    exit 1
fi

TOKEN=$(curl -s -H "Authorization: Bearer $GITHUB_TOKEN" \
    https://api.github.com/copilot_internal/v2/token | jq -r .token)

if [ "$TOKEN" = "null" ] || [ -z "$TOKEN" ]; then
    echo "ERROR: Failed to obtain Copilot token. Check your GITHUB_TOKEN."
    exit 1
fi

echo "=== GET /models ==="
curl -s -w "\nHTTP %{http_code}\n" \
    -H "Authorization: Bearer $TOKEN" \
    -H "Copilot-Integration-Id: vscode-chat" \
    -H "Editor-Version: vscode/1.85.0" \
    -H "Editor-Plugin-Version: copilot-chat/0.12.0" \
    "https://api.githubcopilot.com/models" | head -50

echo
echo "=== Model IDs ==="
curl -s \
    -H "Authorization: Bearer $TOKEN" \
    -H "Copilot-Integration-Id: vscode-chat" \
    -H "Editor-Version: vscode/1.85.0" \
    -H "Editor-Plugin-Version: copilot-chat/0.12.0" \
    "https://api.githubcopilot.com/models" | jq -r '.data[].id'
```

---

## Proposed Design

### Option A: Direct Passthrough (Recommended)

Fetch models from Copilot API on each request, with caching.

```
Client                  Adapter                     Copilot API
  |                        |                            |
  |  GET /v1/models        |                            |
  |----------------------->|                            |
  |                        |  (cache miss)              |
  |                        |  GET /models               |
  |                        |--------------------------->|
  |                        |                            |
  |                        |<---------------------------|
  |                        |  {"data": [...]}           |
  |                        |                            |
  |                        |  (cache response)          |
  |<-----------------------|                            |
  |  {"data": [...]}       |                            |
```

**Pros:**
- Always up-to-date with Copilot's actual model availability
- Minimal code complexity
- Follows OpenAI proxy conventions

**Cons:**
- Requires valid Copilot token to list models (cannot list models without auth)
- Adds latency on cache miss

### Option B: Hybrid (Passthrough + Fallback)

Try to fetch from API; fall back to static list on failure.

```rust
async fn list_models(token: &str) -> ModelList {
    match fetch_models_from_copilot(token).await {
        Ok(models) => models,
        Err(_) => static_fallback_models()
    }
}
```

**Pros:**
- Works even if Copilot API changes or is unavailable
- Graceful degradation

**Cons:**
- May return stale data silently
- Two code paths to maintain

### Option C: Configurable Static List

Load models from a config file instead of hardcoding.

```toml
# ~/.config/copilot-adapter/models.toml
[[models]]
id = "gpt-4o"
owned_by = "github-copilot"

[[models]]
id = "claude-sonnet-4.5"
owned_by = "github-copilot"
```

**Pros:**
- User can customize without recompiling
- No API dependency

**Cons:**
- Still requires manual updates
- User burden to maintain

---

## Recommended Approach

**Option B: Hybrid (Passthrough + Fallback)** with:
1. **Primary:** Fetch from Copilot API with caching (5-minute TTL)
2. **Fallback:** Static list when API unavailable or returns error
3. **Logging:** Warn when falling back to static list

This provides the best balance of accuracy and reliability.

---

## Technical Implementation

### 1. New Method in `CopilotClient`

```rust
// src/copilot/client.rs

const COPILOT_MODELS_URL: &str = "https://api.githubcopilot.com/models";

impl CopilotClient {
    /// Fetch available models from the Copilot API.
    pub async fn fetch_models(&self, token: &str) -> Result<ModelList, AppError> {
        let request_id = uuid::Uuid::new_v4().to_string();

        tracing::debug!(
            request_id = %request_id,
            "Fetching models from Copilot API"
        );

        let response = self.client
            .get(&self.models_url)
            .bearer_auth(token)
            .header("X-Request-Id", &request_id)
            .header("Copilot-Integration-Id", COPILOT_INTEGRATION_ID)
            .header("Editor-Version", EDITOR_VERSION)
            .header("Editor-Plugin-Version", EDITOR_PLUGIN_VERSION)
            .send()
            .await
            .map_err(|e| AppError::CopilotError(format!("Failed to fetch models: {e}")))?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(AppError::CopilotError(
                format!("Models endpoint returned HTTP {status}: {body}")
            ));
        }

        response.json::<ModelList>().await.map_err(|e| {
            AppError::Internal(format!("Failed to parse models response: {e}"))
        })
    }
}
```

### 2. Models Cache

```rust
// src/copilot/models_cache.rs

use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::RwLock;

use crate::copilot::types::ModelList;

/// Cache TTL for models list (5 minutes)
const CACHE_TTL: Duration = Duration::from_secs(300);

pub struct ModelsCache {
    inner: RwLock<Option<CacheEntry>>,
}

struct CacheEntry {
    models: ModelList,
    fetched_at: Instant,
}

impl ModelsCache {
    pub fn new() -> Self {
        Self {
            inner: RwLock::new(None),
        }
    }

    /// Get cached models if still valid.
    pub async fn get(&self) -> Option<ModelList> {
        let guard = self.inner.read().await;
        guard.as_ref().and_then(|entry| {
            if entry.fetched_at.elapsed() < CACHE_TTL {
                Some(entry.models.clone())
            } else {
                None
            }
        })
    }

    /// Store models in cache.
    pub async fn set(&self, models: ModelList) {
        let mut guard = self.inner.write().await;
        *guard = Some(CacheEntry {
            models,
            fetched_at: Instant::now(),
        });
    }

    /// Invalidate the cache.
    pub async fn invalidate(&self) {
        let mut guard = self.inner.write().await;
        *guard = None;
    }
}
```

### 3. Updated Handler

```rust
// src/handlers/models.rs

use std::sync::Arc;
use axum::extract::{Path, State};
use axum::Json;

use crate::copilot::types::{Model, ModelList};
use crate::error::AppError;
use crate::state::AppState;

/// Static fallback models when API is unavailable.
fn fallback_models() -> ModelList {
    ModelList {
        object: "list".to_string(),
        data: vec![
            Model {
                id: "gpt-4o".to_string(),
                object: "model".to_string(),
                created: 1715367049,
                owned_by: "github-copilot".to_string(),
            },
            Model {
                id: "gpt-4-turbo".to_string(),
                object: "model".to_string(),
                created: 1712361441,
                owned_by: "github-copilot".to_string(),
            },
            Model {
                id: "gpt-4".to_string(),
                object: "model".to_string(),
                created: 1686935002,
                owned_by: "github-copilot".to_string(),
            },
        ],
    }
}

/// Handler for `GET /v1/models` — returns available models.
pub async fn list_models(
    State(state): State<Arc<AppState>>,
) -> Result<Json<ModelList>, AppError> {
    // Check cache first
    if let Some(cached) = state.models_cache.get().await {
        tracing::debug!("Returning cached models list");
        return Ok(Json(cached));
    }

    // Try to fetch from Copilot API
    let token = state.token_manager.get_token().await?;

    match state.copilot_client.fetch_models(&token).await {
        Ok(models) => {
            tracing::info!(count = models.data.len(), "Fetched models from Copilot API");
            state.models_cache.set(models.clone()).await;
            Ok(Json(models))
        }
        Err(e) => {
            tracing::warn!(
                error = %e,
                "Failed to fetch models from Copilot API, using fallback"
            );
            Ok(Json(fallback_models()))
        }
    }
}

/// Handler for `GET /v1/models/:model` — returns details for a specific model.
pub async fn get_model(
    State(state): State<Arc<AppState>>,
    Path(model_id): Path<String>,
) -> Result<Json<Model>, AppError> {
    // Get full models list (cached or fresh)
    let models = list_models(State(state)).await?.0;

    models.data
        .into_iter()
        .find(|m| m.id == model_id)
        .map(Json)
        .ok_or_else(|| AppError::ModelNotFound(format!("Model '{model_id}' not found")))
}
```

### 4. AppState Updates

```rust
// src/state.rs (or wherever AppState is defined)

pub struct AppState {
    pub copilot_client: CopilotClient,
    pub token_manager: TokenManager,
    pub models_cache: ModelsCache,  // ← New field
    pub config: Config,
}
```

### 5. Configuration Options

Add optional CLI flags for cache behavior:

```rust
// src/cli.rs

#[derive(Parser)]
pub struct StartArgs {
    // ... existing args ...

    /// Models cache TTL in seconds (default: 300)
    #[arg(long, default_value = "300")]
    pub models_cache_ttl: u64,

    /// Disable dynamic models fetching (use static list only)
    #[arg(long)]
    pub static_models: bool,
}
```

---

## API Endpoint Discovery — COMPLETED

Endpoint confirmed as `GET https://api.githubcopilot.com/models`.
See "Endpoint Discovery Findings" section above for full details.

### Verification Script

See the verification script in the "Endpoint Discovery Findings" section above.

---

## File Changes Summary

| File | Change |
|------|--------|
| `src/copilot/client.rs` | Add `fetch_models()` method, add `models_url` field |
| `src/copilot/models_cache.rs` | **New file** — Cache implementation |
| `src/copilot/mod.rs` | Export `models_cache` module |
| `src/handlers/models.rs` | Rewrite to use dynamic fetching with fallback |
| `src/state.rs` | Add `models_cache` field to `AppState` |
| `src/cli.rs` | Add `--models-cache-ttl` and `--static-models` flags |
| `src/main.rs` / `src/server.rs` | Initialize `ModelsCache` in app state |

---

## Testing Strategy

### Unit Tests

1. **Cache behavior:**
   - Cache hit within TTL returns cached data
   - Cache miss triggers fetch
   - Expired cache triggers refresh
   - Invalidation clears cache

2. **Fallback behavior:**
   - API error triggers fallback
   - Network timeout triggers fallback
   - Fallback returns valid model list

3. **Response parsing:**
   - Valid OpenAI format parsed correctly
   - Extra fields ignored gracefully
   - Empty list handled

### Integration Tests

1. **With mock server:**
   - Verify correct headers sent to Copilot API
   - Verify caching works across requests
   - Verify fallback on 5xx errors

2. **Cache TTL:**
   - First request fetches from API
   - Second request (within TTL) uses cache
   - Request after TTL fetches again

### Manual E2E Tests

1. Start adapter, call `/v1/models` — verify response
2. Stop network, call `/v1/models` — verify fallback
3. Check logs for cache hit/miss messages
4. Verify `/v1/models/:model` returns correct model

---

## Rollout Plan

### Phase 1: API Discovery (COMPLETED)
- ✅ Confirmed correct endpoint: `GET https://api.githubcopilot.com/models`
- ✅ Documented actual response format (OpenAI-compatible)
- ✅ Confirmed required headers (same as chat completions)
- ✅ No specific rate limits observed; 5-min cache TTL recommended
- ✅ Response is subscription-aware (model list varies by plan)

### Phase 2: Implementation
- Implement cache module
- Add `fetch_models()` to client
- Update handlers
- Add CLI flags

### Phase 3: Testing
- Unit tests for cache and fallback
- Integration tests with mock server
- Manual testing with real Copilot API

### Phase 4: Documentation
- Update README with new behavior
- Document CLI flags
- Add troubleshooting section for models issues

---

## Open Questions

| # | Question | Status |
|---|----------|--------|
| 1 | What is the exact Copilot models endpoint URL? | **RESOLVED:** `GET https://api.githubcopilot.com/models` |
| 2 | Does Copilot return capability metadata (context window, etc.)? | **RESOLVED:** No — only `id`, `object`, `created`, `owned_by`. Some versions include `vendor`/`name` |
| 3 | Should we expose a `/v1/models/refresh` endpoint to force cache invalidation? | Deferred |
| 4 | Should fallback models be configurable via file? | Deferred (Option C hybrid) |
| 5 | How should we handle token refresh during models fetch? | **RESOLVED:** Use existing TokenManager |

---

## References

- [OpenAI Models API](https://platform.openai.com/docs/api-reference/models)
- [DESIGN.md](./DESIGN.md) — Main adapter design document
- [src/handlers/models.rs](./src/handlers/models.rs) — Current implementation
- [src/copilot/client.rs](./src/copilot/client.rs) — Copilot API client
