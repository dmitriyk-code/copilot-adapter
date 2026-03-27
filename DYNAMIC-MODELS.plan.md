# Dynamic Models List — Implementation Plan

**Status:** Draft (Epic 0 Complete)
**Date:** 2026-03-27
**Based on:** [DYNAMIC-MODELS.design.md](./DYNAMIC-MODELS.design.md)
**Prerequisite:** Core adapter implementation — COMPLETE

---

## Executive Summary

This plan implements dynamic model list fetching for the GitHub Copilot Adapter. Currently, the `/v1/models` endpoint returns a hardcoded list of models that may be outdated (e.g., listing retired models like `claude-3.5-sonnet`). This feature adds:

- **Dynamic fetching** from GitHub Copilot's models API (default behavior)
- **In-memory caching** with configurable TTL (default: 5 minutes)
- **Graceful fallback** to a static model list when the API is unavailable
- **CLI flags** for cache TTL and static-only mode

This ensures users always see accurate model availability without manual adapter updates.

---

## Background

### Current State

The `/v1/models` endpoint (in `src/handlers/models.rs`) returns a hardcoded list:
- `gpt-4`, `gpt-4o`, `gpt-4-turbo`, `gpt-3.5-turbo`, `claude-3.5-sonnet`

Problems:
1. `claude-3.5-sonnet` was retired (Feb 2026) but still appears
2. Newer models (GPT-5.x, Claude 4.x) are missing
3. Requires code changes to update the list

### Target State

- `/v1/models` fetches from Copilot API by default
- Results are cached for 5 minutes (configurable)
- If API fails, falls back to static list with warning log
- New CLI flags: `--models-cache-ttl`, `--static-models`

---

## Problem Statement

Users querying `/v1/models` receive inaccurate information:
1. **Retired models listed** — May cause requests to fail with model-not-found errors
2. **New models missing** — Users unaware of available options
3. **Manual maintenance burden** — Every model change requires adapter update and recompilation

---

## Goals and Non-Goals

### Goals

| ID | Goal | Success Criteria |
|----|------|------------------|
| G1 | Fetch models dynamically from Copilot API | `/v1/models` returns real-time data when API available |
| G2 | Cache model list to reduce API calls | Subsequent requests within TTL use cached data |
| G3 | Graceful fallback on API failure | Returns static list when API unavailable; logs warning |
| G4 | Configurable cache TTL | `--models-cache-ttl` flag controls cache duration |
| G5 | Static-only mode | `--static-models` flag disables dynamic fetching |
| G6 | Backward compatible | Existing API consumers see no breaking changes |

### Non-Goals

| ID | Non-Goal | Rationale |
|----|----------|-----------|
| NG1 | Persistent cache (disk) | In-memory sufficient; restart refreshes naturally |
| NG2 | Model capability metadata | Depends on Copilot API response; out of scope |
| NG3 | Manual cache refresh endpoint | Low value; users can restart adapter |
| NG4 | Configurable fallback list | Static list in code is sufficient for fallback |

---

## Requirements

### Functional Requirements

| ID | Requirement | Source |
|----|-------------|--------|
| FR1 | `GET /v1/models` fetches from Copilot API when cache is empty/expired | Design: Option B |
| FR2 | Model list cached in memory with configurable TTL | Design: Cache section |
| FR3 | Cache miss triggers API fetch; cache hit returns cached data | Design: Cache section |
| FR4 | API failure falls back to static model list | Design: Option B |
| FR5 | Fallback logs warning message | Design: Recommended Approach |
| FR6 | `--models-cache-ttl <seconds>` flag sets cache duration | Design: Configuration |
| FR7 | `--static-models` flag disables dynamic fetching | Design: Configuration |
| FR8 | `GET /v1/models/:model` uses same cache/fallback logic | Design: Handler section |
| FR9 | Token required for API fetch; uses existing TokenManager | Design: Implementation |

### Non-Functional Requirements

| ID | Requirement | Target |
|----|-------------|--------|
| NFR1 | Cache lookup latency | < 1ms |
| NFR2 | API fetch latency overhead | < 500ms (network dependent) |
| NFR3 | Memory overhead for cache | < 1KB (model list is small) |
| NFR4 | Concurrent request handling | Cache is thread-safe via RwLock |

---

## Proposed Architecture

### Component Overview

```
┌─────────────────────────────────────────────────────────────────────┐
│                        copilot-adapter                               │
│                                                                     │
│  ┌───────────────┐     ┌─────────────────────────────────────────┐  │
│  │ CLI (clap)    │     │ AppState                                │  │
│  │ + models_     │     │ + token_manager: TokenManager           │  │
│  │   cache_ttl   │     │ + copilot_client: CopilotClient         │  │
│  │ + static_     │     │ + models_cache: ModelsCache    ◄─ NEW   │  │
│  │   models      │     │ + config: AdapterConfig                 │  │
│  └───────────────┘     └─────────────────────────────────────────┘  │
│                                                                     │
│  ┌───────────────────────────────────────────────────────────────┐  │
│  │ Handlers                                                      │  │
│  │                                                               │  │
│  │  GET /v1/models ──────► ModelsCache.get() ──┬─► Cache HIT     │  │
│  │                                             │      │          │  │
│  │                                             │      ▼          │  │
│  │                                             │   Return cached │  │
│  │                                             │                 │  │
│  │                                             └─► Cache MISS    │  │
│  │                                                    │          │  │
│  │                                                    ▼          │  │
│  │                                        CopilotClient.fetch()  │  │
│  │                                                    │          │  │
│  │                                           ┌───────┴───────┐   │  │
│  │                                           ▼               ▼   │  │
│  │                                       SUCCESS          FAILURE│  │
│  │                                           │               │   │  │
│  │                                           ▼               ▼   │  │
│  │                                   Cache + Return    Fallback  │  │
│  └───────────────────────────────────────────────────────────────┘  │
│                                                                     │
│  ┌───────────────────────────────────────────────────────────────┐  │
│  │ src/copilot/models_cache.rs  ◄─ NEW FILE                      │  │
│  │                                                               │  │
│  │  pub struct ModelsCache {                                     │  │
│  │      inner: RwLock<Option<CacheEntry>>,                       │  │
│  │      ttl: Duration,                                           │  │
│  │  }                                                            │  │
│  │                                                               │  │
│  │  impl ModelsCache {                                           │  │
│  │      pub fn new(ttl: Duration) -> Self                        │  │
│  │      pub async fn get(&self) -> Option<ModelList>             │  │
│  │      pub async fn set(&self, models: ModelList)               │  │
│  │      pub async fn invalidate(&self)                           │  │
│  │  }                                                            │  │
│  └───────────────────────────────────────────────────────────────┘  │
│                                                                     │
│  ┌───────────────────────────────────────────────────────────────┐  │
│  │ src/copilot/client.rs  (MODIFIED)                             │  │
│  │                                                               │  │
│  │  impl CopilotClient {                                         │  │
│  │      pub async fn fetch_models(&self, token: &str)            │  │
│  │          -> Result<ModelList, AppError>   ◄─ NEW METHOD       │  │
│  │  }                                                            │  │
│  └───────────────────────────────────────────────────────────────┘  │
└─────────────────────────────────────────────────────────────────────┘
                              │
                              ▼ HTTPS
┌─────────────────────────────────────────────────────────────────────┐
│                    GitHub Copilot API                                │
│                                                                     │
│  GET /models  ← confirmed endpoint                                  │
│  Authorization: Bearer <copilot_token>                              │
└─────────────────────────────────────────────────────────────────────┘
```

### Data Flow

```
GET /v1/models request
        │
        ▼
┌───────────────────────────────┐
│ Check --static-models flag    │
│ If enabled → return fallback  │
└───────────────────────────────┘
        │ dynamic enabled
        ▼
┌───────────────────────────────┐
│ ModelsCache.get()             │
│ • Check if cache entry exists │
│ • Check if entry within TTL   │
└───────────────────────────────┘
        │
   ┌────┴────┐
   ▼         ▼
 HIT       MISS
   │         │
   │         ▼
   │    ┌───────────────────────────────┐
   │    │ TokenManager.get_token()      │
   │    │ • Get valid Copilot token     │
   │    └───────────────────────────────┘
   │         │
   │         ▼
   │    ┌───────────────────────────────┐
   │    │ CopilotClient.fetch_models()  │
   │    │ • GET https://api.github      │
   │    │   copilot.com/models          │
   │    │ • Parse ModelList response    │
   │    └───────────────────────────────┘
   │         │
   │    ┌────┴────┐
   │    ▼         ▼
   │  SUCCESS   FAILURE
   │    │         │
   │    ▼         ▼
   │  Cache     Log warning
   │  result    Return fallback
   │    │
   └────┼─────────┘
        ▼
   Return ModelList
```

### Key Design Decisions

| Decision | Rationale |
|----------|-----------|
| **In-memory cache only** | Simple; models list is small; restart refreshes naturally |
| **RwLock for thread safety** | Allows concurrent reads; single writer for updates |
| **Fallback on any error** | Network issues, auth errors, parsing errors all trigger fallback |
| **Warning log on fallback** | User aware of degraded state without request failure |
| **TTL configurable via CLI** | Different use cases may need different freshness |
| **Static mode opt-in** | Users in restricted networks can disable API calls |

---

## Dependencies

### New Dependencies

None required. Uses existing:
- `tokio::sync::RwLock` for thread-safe cache
- `std::time::{Duration, Instant}` for TTL tracking
- `reqwest` for HTTP (already in CopilotClient)

### Sequencing Constraints

1. **Epic 1** (API Discovery) should complete first to confirm endpoint URL
2. **Epic 2** (Cache) and **Epic 3** (Client) can proceed in parallel after Epic 1
3. **Epic 4** (Handler) depends on Epics 2 and 3
4. **Epic 5** (CLI) can proceed in parallel with Epic 4
5. **Epic 6** (Testing) depends on all previous epics

---

## Impact Analysis

### Files Modified

| File Path | Changes |
|-----------|---------|
| `src/copilot/client.rs` | Add `fetch_models()` method; add `models_url` field |
| `src/copilot/mod.rs` | Export `models_cache` module |
| `src/handlers/models.rs` | Rewrite to use dynamic fetching with cache and fallback |
| `src/server.rs` | Add `models_cache: ModelsCache` to `AppState` |
| `src/cli.rs` | Add `--models-cache-ttl` and `--static-models` flags to `Start` command |
| `src/main.rs` | Initialize `ModelsCache` with configured TTL; pass to server |

### Files Created

| File Path | Purpose |
|-----------|---------|
| `src/copilot/models_cache.rs` | Cache implementation with TTL |
| `tests/unit/models_cache_tests.rs` | Cache behavior unit tests |
| `tests/integration/models_dynamic_tests.rs` | Integration tests with mock server |

---

## Risks and Mitigations

| # | Risk | Likelihood | Impact | Mitigation |
|---|------|------------|--------|------------|
| R1 | Copilot doesn't expose `/models` endpoint | ~~Medium~~ **RESOLVED** | High | ✅ Endpoint confirmed: `GET /models` works |
| R2 | API response format differs from OpenAI | Low | Medium | Parse flexibly; log and fallback on parse error. Discovery confirmed OpenAI-compatible format with possible extra fields (`vendor`, `name`) |
| R3 | Token refresh race during models fetch | Low | Low | TokenManager already handles refresh; retry once |
| R4 | Cache stampede on cold start | Low | Low | Single fetch; others wait on RwLock |
| R5 | Stale fallback list | Medium | Low | Update fallback list periodically; document limitation |

---

## Implementation Plan

### Epic 0: API Endpoint Discovery

**Goal:** Confirm the correct Copilot models API endpoint and response format.

**Prerequisites:** Valid Copilot authentication

**Status:** DONE

**Tasks:**

| Task ID | Type | Description | Status |
|---------|------|-------------|--------|
| E0-T1 | RESEARCH | Test `GET https://api.githubcopilot.com/models` with valid token | DONE |
| E0-T2 | RESEARCH | Test `GET https://api.githubcopilot.com/v1/models` with valid token | DONE |
| E0-T3 | RESEARCH | Document response format and available fields | DONE |
| E0-T4 | RESEARCH | Check for rate limits or special requirements | DONE |
| E0-T5 | DOC | Update DYNAMIC-MODELS.design.md with findings | DONE |

**Acceptance Criteria:**
- [x] Correct endpoint URL identified
- [x] Response format documented
- [x] Required headers confirmed
- [x] Rate limit behavior understood

**Findings Summary:**

The correct endpoint is `GET https://api.githubcopilot.com/models` (no `/v1` prefix).
See DYNAMIC-MODELS.design.md "Endpoint Discovery Findings" section for full details.

**Verification Script:**
```bash
#!/bin/bash
# Run after `copilot-adapter auth`
#
# NOTE: The adapter stores credentials in the OS keyring (with encrypted file
# fallback), NOT as a plain-text file. Supply your GitHub PAT via GITHUB_TOKEN:
#
#   export GITHUB_TOKEN="ghp_your_personal_access_token"

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

# Test confirmed endpoint
echo "=== GET /models (confirmed endpoint) ==="
curl -s -w "\nHTTP %{http_code}\n" \
    -H "Authorization: Bearer $TOKEN" \
    -H "Copilot-Integration-Id: vscode-chat" \
    -H "Editor-Version: vscode/1.85.0" \
    -H "Editor-Plugin-Version: copilot-chat/0.12.0" \
    "https://api.githubcopilot.com/models" | head -50
echo
```

---

### Epic 1: Models Cache Implementation

**Goal:** Create the in-memory cache module with TTL support.

**Prerequisites:** None (can start immediately)

**Status:** DONE

**Tasks:**

| Task ID | Type | Description | Files | Status |
|---------|------|-------------|-------|--------|
| E1-T1 | IMPL | Create `src/copilot/models_cache.rs` with `ModelsCache` struct | `src/copilot/models_cache.rs` | DONE |
| E1-T2 | IMPL | Implement `CacheEntry` struct with `models: ModelList` and `fetched_at: Instant` | `src/copilot/models_cache.rs` | DONE |
| E1-T3 | IMPL | Implement `ModelsCache::new(ttl: Duration)` constructor | `src/copilot/models_cache.rs` | DONE |
| E1-T4 | IMPL | Implement `ModelsCache::get()` — returns `Option<ModelList>` if within TTL | `src/copilot/models_cache.rs` | DONE |
| E1-T5 | IMPL | Implement `ModelsCache::set(models: ModelList)` — stores with current timestamp | `src/copilot/models_cache.rs` | DONE |
| E1-T6 | IMPL | Implement `ModelsCache::invalidate()` — clears cache | `src/copilot/models_cache.rs` | DONE |
| E1-T7 | IMPL | Export module from `src/copilot/mod.rs` | `src/copilot/mod.rs` | DONE |
| E1-T8 | TEST | Unit test: `get()` returns `None` on empty cache | `tests/unit/models_cache_tests.rs` | DONE |
| E1-T9 | TEST | Unit test: `set()` then `get()` returns cached data | `tests/unit/models_cache_tests.rs` | DONE |
| E1-T10 | TEST | Unit test: `get()` returns `None` after TTL expires | `tests/unit/models_cache_tests.rs` | DONE |
| E1-T11 | TEST | Unit test: `invalidate()` clears cache | `tests/unit/models_cache_tests.rs` | DONE |
| E1-T12 | TEST | Unit test: concurrent reads don't block | `tests/unit/models_cache_tests.rs` | DONE |

**Acceptance Criteria:**
- [x] Cache stores and retrieves `ModelList` correctly
- [x] TTL expiration works as expected
- [x] Thread-safe for concurrent access
- [x] All unit tests pass

---

### Epic 2: CopilotClient Models Fetch

**Goal:** Add method to fetch models from Copilot API.

**Prerequisites:** Epic 0 (endpoint discovery)

**Status:** TODO

**Tasks:**

| Task ID | Type | Description | Files | Status |
|---------|------|-------------|-------|--------|
| E2-T1 | IMPL | Add `COPILOT_MODELS_URL` constant (based on Epic 0 findings) | `src/copilot/client.rs` | TODO |
| E2-T2 | IMPL | Add `models_url: String` field to `CopilotClient` | `src/copilot/client.rs` | TODO |
| E2-T3 | IMPL | Update `CopilotClient::new()` to set default models URL | `src/copilot/client.rs` | TODO |
| E2-T4 | IMPL | Update `CopilotClient::with_api_url()` to accept optional models URL | `src/copilot/client.rs` | TODO |
| E2-T5 | IMPL | Implement `fetch_models(&self, token: &str) -> Result<ModelList, AppError>` | `src/copilot/client.rs` | TODO |
| E2-T6 | IMPL | Add appropriate request headers (same as chat completions) | `src/copilot/client.rs` | TODO |
| E2-T7 | IMPL | Handle non-2xx responses with `AppError::CopilotError` | `src/copilot/client.rs` | TODO |
| E2-T8 | IMPL | Parse response as `ModelList` | `src/copilot/client.rs` | TODO |
| E2-T9 | TEST | Unit test: successful fetch returns ModelList | `tests/unit/copilot_client_tests.rs` | TODO |
| E2-T10 | TEST | Unit test: 404 returns CopilotError | `tests/unit/copilot_client_tests.rs` | TODO |
| E2-T11 | TEST | Unit test: 401 returns appropriate error | `tests/unit/copilot_client_tests.rs` | TODO |
| E2-T12 | TEST | Unit test: malformed JSON returns parse error | `tests/unit/copilot_client_tests.rs` | TODO |

**Acceptance Criteria:**
- [ ] `fetch_models()` sends correct request to Copilot API
- [ ] Response parsed into `ModelList` type
- [ ] Errors handled gracefully with appropriate `AppError` variants
- [ ] All unit tests pass

---

### Epic 3: Handler Integration

**Goal:** Integrate dynamic fetching into `/v1/models` handlers.

**Prerequisites:** Epics 1 and 2

**Status:** TODO

**Tasks:**

| Task ID | Type | Description | Files | Status |
|---------|------|-------------|-------|--------|
| E3-T1 | IMPL | Add `models_cache: ModelsCache` field to `AppState` | `src/server.rs` | TODO |
| E3-T2 | IMPL | Add `static_models: bool` field to `AdapterConfig` | `src/server.rs` | TODO |
| E3-T3 | IMPL | Create `fallback_models()` function returning static `ModelList` | `src/handlers/models.rs` | TODO |
| E3-T4 | IMPL | Update `list_models()` handler signature to accept `State<Arc<AppState>>` | `src/handlers/models.rs` | TODO |
| E3-T5 | IMPL | Implement cache check: return cached if valid | `src/handlers/models.rs` | TODO |
| E3-T6 | IMPL | Implement API fetch on cache miss | `src/handlers/models.rs` | TODO |
| E3-T7 | IMPL | Implement fallback with warning log on fetch error | `src/handlers/models.rs` | TODO |
| E3-T8 | IMPL | Handle `--static-models` flag: skip fetch, use fallback | `src/handlers/models.rs` | TODO |
| E3-T9 | IMPL | Update `get_model()` to use same logic | `src/handlers/models.rs` | TODO |
| E3-T10 | IMPL | Update router to pass `State` to models handlers | `src/server.rs` | TODO |
| E3-T11 | TEST | Integration test: cache hit returns cached data | `tests/integration/models_dynamic_tests.rs` | TODO |
| E3-T12 | TEST | Integration test: cache miss fetches from API | `tests/integration/models_dynamic_tests.rs` | TODO |
| E3-T13 | TEST | Integration test: API error triggers fallback | `tests/integration/models_dynamic_tests.rs` | TODO |
| E3-T14 | TEST | Integration test: static mode always uses fallback | `tests/integration/models_dynamic_tests.rs` | TODO |
| E3-T15 | TEST | Integration test: get_model with valid ID succeeds | `tests/integration/models_dynamic_tests.rs` | TODO |
| E3-T16 | TEST | Integration test: get_model with invalid ID returns 404 | `tests/integration/models_dynamic_tests.rs` | TODO |

**Acceptance Criteria:**
- [ ] `/v1/models` returns dynamic data when API available
- [ ] `/v1/models` returns fallback when API unavailable
- [ ] Warning logged when falling back
- [ ] `/v1/models/:model` works correctly
- [ ] `--static-models` bypasses dynamic fetch
- [ ] All integration tests pass

---

### Epic 4: CLI Configuration

**Goal:** Add CLI flags for cache TTL and static mode.

**Prerequisites:** Epic 3

**Status:** TODO

**Tasks:**

| Task ID | Type | Description | Files | Status |
|---------|------|-------------|-------|--------|
| E4-T1 | IMPL | Add `--models-cache-ttl <seconds>` flag to `StartArgs` (default: 300) | `src/cli.rs` | TODO |
| E4-T2 | IMPL | Add `--static-models` flag to `StartArgs` (default: false) | `src/cli.rs` | TODO |
| E4-T3 | IMPL | Pass TTL to `ModelsCache::new()` in main.rs | `src/main.rs` | TODO |
| E4-T4 | IMPL | Pass `static_models` to `AdapterConfig` in main.rs | `src/main.rs` | TODO |
| E4-T5 | IMPL | Add help text for both flags | `src/cli.rs` | TODO |
| E4-T6 | TEST | CLI test: default TTL is 300 seconds | Manual | TODO |
| E4-T7 | TEST | CLI test: custom TTL is respected | Manual | TODO |
| E4-T8 | TEST | CLI test: `--static-models` disables fetching | Manual | TODO |

**Acceptance Criteria:**
- [ ] `--models-cache-ttl` flag accepted and parsed
- [ ] `--static-models` flag accepted and parsed
- [ ] Help text describes both flags
- [ ] Default values work correctly

---

### Epic 5: Testing and Documentation

**Goal:** Comprehensive testing and documentation updates.

**Prerequisites:** Epics 1-4

**Status:** TODO

**Tasks:**

| Task ID | Type | Description | Files | Status |
|---------|------|-------------|-------|--------|
| E5-T1 | TEST | Create mock Copilot models endpoint for tests | `tests/common/mock_copilot.rs` | TODO |
| E5-T2 | TEST | E2E test: fresh start fetches from API | Manual | TODO |
| E5-T3 | TEST | E2E test: second request within TTL uses cache | Manual | TODO |
| E5-T4 | TEST | E2E test: request after TTL refetches | Manual | TODO |
| E5-T5 | TEST | E2E test: network disconnect triggers fallback | Manual | TODO |
| E5-T6 | DOC | Update README.md with dynamic models documentation | `README.md` | TODO |
| E5-T7 | DOC | Document `--models-cache-ttl` and `--static-models` flags | `README.md` | TODO |
| E5-T8 | DOC | Update CLAUDE.md with models feature notes | `CLAUDE.md` | TODO |
| E5-T9 | DOC | Add models section to docs/e2e-testing.md | `docs/e2e-testing.md` | TODO |
| E5-T10 | DOC | Update DYNAMIC-MODELS.design.md status to "Implemented" | `DYNAMIC-MODELS.design.md` | TODO |

**Acceptance Criteria:**
- [ ] All unit tests pass
- [ ] All integration tests pass
- [ ] E2E scenarios verified manually
- [ ] README documents the feature
- [ ] CLAUDE.md updated with notes

---

## Verification Plan

After implementation, verify dynamic models work correctly:

1. **API Discovery Test** (Epic 0)
   ```bash
   # Manually test Copilot models endpoint
   ./test_models_endpoint.sh
   ```

2. **Basic Functionality Test**
   ```bash
   copilot-adapter start
   curl http://127.0.0.1:6767/v1/models
   # Should return models from Copilot API
   ```

3. **Cache Test**
   ```bash
   copilot-adapter start --log-level debug
   curl http://127.0.0.1:6767/v1/models  # First request - API fetch
   curl http://127.0.0.1:6767/v1/models  # Second request - cache hit
   # Check logs for "cache hit" vs "fetching from API"
   ```

4. **TTL Test**
   ```bash
   copilot-adapter start --models-cache-ttl 10  # 10 second TTL
   curl http://127.0.0.1:6767/v1/models  # Fetch
   sleep 11
   curl http://127.0.0.1:6767/v1/models  # Should refetch
   ```

5. **Fallback Test**
   ```bash
   # Disconnect network or use invalid token
   copilot-adapter start
   curl http://127.0.0.1:6767/v1/models
   # Should return fallback list with warning in logs
   ```

6. **Static Mode Test**
   ```bash
   copilot-adapter start --static-models
   curl http://127.0.0.1:6767/v1/models
   # Should return static list without API call
   ```

7. **Get Model Test**
   ```bash
   curl http://127.0.0.1:6767/v1/models/gpt-4o
   # Should return model details

   curl http://127.0.0.1:6767/v1/models/nonexistent
   # Should return 404
   ```

---

## Fallback Model List

The static fallback list should include commonly available models. Update as needed:

```rust
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
            Model {
                id: "gpt-3.5-turbo".to_string(),
                object: "model".to_string(),
                created: 1677649963,
                owned_by: "github-copilot".to_string(),
            },
        ],
    }
}
```

**Note:** Removed `claude-3.5-sonnet` from fallback as it was retired Feb 2026.

---

## Open Questions

| # | Question | Resolution |
|---|----------|------------|
| 1 | What is the exact Copilot models endpoint? | Pending Epic 0 discovery |
| 2 | Does Copilot return model capabilities (context window, etc.)? | Pending Epic 0 discovery |
| 3 | Should we retry on transient errors before fallback? | Start with immediate fallback; add retry if needed |
| 4 | Should cache be shared across adapter restarts (persistent)? | No — in-memory sufficient |

---

## References

| Document | Description |
|----------|-------------|
| [DYNAMIC-MODELS.design.md](./DYNAMIC-MODELS.design.md) | Design document |
| [OpenAI Models API](https://platform.openai.com/docs/api-reference/models) | Reference API format |
| [src/handlers/models.rs](./src/handlers/models.rs) | Current implementation |
| [src/copilot/client.rs](./src/copilot/client.rs) | Copilot API client |
| [IMPLEMENTATION.plan.md](./IMPLEMENTATION.plan.md) | Main adapter implementation plan |
