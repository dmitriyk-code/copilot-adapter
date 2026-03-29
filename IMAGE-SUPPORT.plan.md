# Image and Document Content Block Support — Implementation Plan

**Status:** In Progress
**Date:** 2026-03-29
**Based on:** [IMAGE-SUPPORT.design.md](./IMAGE-SUPPORT.design.md)
**Prerequisite:** Core adapter implementation, Anthropic Messages API support — COMPLETE

---

## Executive Summary

This plan implements image and document content block support for the Anthropic Messages API endpoint (`/v1/messages`) in copilot-adapter. Currently, the adapter fails with a 422 deserialization error when Claude Code uploads an image because the `ContentBlock` enum only supports `text`, `tool_use`, and `tool_result` variants.

This feature adds:

- **Image block support** with translation to OpenAI's `image_url` format
- **Document block support** with graceful degradation (skip with warning)
- **Cache control metadata** (accepted but ignored)
- **Backward compatibility** for existing text and tool blocks

**Validated Approach**: This implementation matches LiteLLM's proven working solution for Claude Code → GitHub Copilot with image uploads.

---

## Background

### Current State

The `ContentBlock` enum in `src/anthropic/types.rs` (lines 50-67) only supports:
- `text` - plain text content
- `tool_use` - tool invocation blocks
- `tool_result` - tool response blocks

When Claude Code sends an image:
```json
{
  "type": "image",
  "source": {
    "type": "base64",
    "media_type": "image/jpeg",
    "data": "..."
  }
}
```

The deserialization fails:
```
Failed to deserialize the JSON body into the target type: messages[14].content:
data did not match any variant of untagged enum ContentBlockInput
```

### Target State

- Anthropic `ContentBlock` enum supports `image`, `document`, and optional `cache_control`
- OpenAI `ContentBlock` enum supports `image_url` for multimodal content
- Translation logic converts Anthropic image blocks → OpenAI image_url blocks with data URIs
- Document blocks are gracefully skipped with warning logs
- Vision-capable models (GPT-4o, Claude 3.5) can receive images through the adapter

---

## Problem Statement

Users cannot upload images through Claude Code when using copilot-adapter:
1. **Deserialization fails** — Image blocks cause 422 errors
2. **Vision features unavailable** — Models with vision capabilities cannot be used
3. **API incompatibility** — Adapter doesn't fully support Anthropic Messages API spec

---

## Goals and Non-Goals

### Goals

| ID | Goal | Success Criteria |
|----|------|------------------|
| G1 | Support Anthropic image content blocks | Image blocks deserialize successfully |
| G2 | Translate images to OpenAI format | Images converted to `image_url` with data URIs |
| G3 | Support document content blocks | Document blocks deserialize (even if skipped) |
| G4 | Graceful degradation for documents | Documents skipped with warning log, request succeeds |
| G5 | Cache control metadata support | Cache control fields accepted but ignored |
| G6 | Backward compatible | Existing text/tool blocks continue working |

### Non-Goals

| ID | Non-Goal | Rationale |
|----|----------|-----------|
| NG1 | Document-to-text extraction | No OpenAI equivalent; complex; low priority |
| NG2 | Extended block types (thinking, search_result, etc.) | Not needed for image upload; can add later |
| NG3 | Cache control translation | OpenAI doesn't support caching |
| NG4 | Streaming support for images | Static content; no streaming needed |

---

## Requirements

### Functional Requirements

| ID | Requirement | Source |
|----|-------------|--------|
| FR1 | Deserialize Anthropic image blocks with base64 sources | Anthropic API spec |
| FR2 | Deserialize Anthropic image blocks with URL sources | Anthropic API spec |
| FR3 | Deserialize Anthropic document blocks | Anthropic API spec |
| FR4 | Translate image blocks to OpenAI `image_url` format | OpenAI API spec |
| FR5 | Convert base64 images to data URIs | OpenAI multimodal format |
| FR6 | Pass through URL images unchanged | OpenAI multimodal format |
| FR7 | Skip document blocks with warning log | Design decision |
| FR8 | Accept `cache_control` on all block types | Anthropic API spec |
| FR9 | Ignore `cache_control` during translation | OpenAI limitation |
| FR10 | Support mixed content (text + images) in single message | Both API specs |

### Non-Functional Requirements

| ID | Requirement | Target |
|----|-------------|--------|
| NFR1 | No performance regression for text-only messages | < 1ms overhead |
| NFR2 | Base64 encoding overhead | Acceptable (user-provided) |
| NFR3 | Memory overhead for image data | Temporary (request lifetime) |
| NFR4 | Backward compatibility | 100% (no breaking changes) |

---

## Proposed Architecture

### Component Overview

```
┌─────────────────────────────────────────────────────────────────────┐
│                        copilot-adapter                               │
│                                                                     │
│  ┌───────────────────────────────────────────────────────────────┐  │
│  │ src/anthropic/types.rs  (MODIFIED)                            │  │
│  │                                                               │  │
│  │  pub enum ContentBlock {                                      │  │
│  │      Text { text, cache_control? },          ◄─ ADD cache     │  │
│  │      Image { source, cache_control? },       ◄─ NEW           │  │
│  │      Document { source, title?, cache_control? }, ◄─ NEW      │  │
│  │      ToolUse { id, name, input, cache_control? }, ◄─ ADD      │  │
│  │      ToolResult { tool_use_id, content, cache_control? },     │  │
│  │  }                                                            │  │
│  │                                                               │  │
│  │  pub enum ImageSource {            ◄─ NEW                     │  │
│  │      Base64 { media_type, data },                             │  │
│  │      Url { media_type, url },                                 │  │
│  │  }                                                            │  │
│  │                                                               │  │
│  │  pub enum DocumentSource {         ◄─ NEW                     │  │
│  │      Base64 { media_type, data },                             │  │
│  │      Text { media_type, data },                               │  │
│  │      Url { media_type, url },                                 │  │
│  │  }                                                            │  │
│  │                                                               │  │
│  │  pub struct CacheControl {         ◄─ NEW                     │  │
│  │      cache_type: String,                                      │  │
│  │      ttl: Option<String>,                                     │  │
│  │  }                                                            │  │
│  │                                                               │  │
│  │  fn translate_content_block(...)   ◄─ NEW                     │  │
│  └───────────────────────────────────────────────────────────────┘  │
│                                                                     │
│  ┌───────────────────────────────────────────────────────────────┐  │
│  │ src/copilot/types.rs  (MODIFIED)                              │  │
│  │                                                               │  │
│  │  pub enum ContentBlock {                                      │  │
│  │      Text { text },                                           │  │
│  │      ImageUrl { image_url },       ◄─ NEW                     │  │
│  │      Other,                                                   │  │
│  │  }                                                            │  │
│  │                                                               │  │
│  │  pub struct ImageUrl {             ◄─ NEW                     │  │
│  │      url: String,                                             │  │
│  │      detail: Option<String>,                                  │  │
│  │  }                                                            │  │
│  └───────────────────────────────────────────────────────────────┘  │
└─────────────────────────────────────────────────────────────────────┘
```

### Data Flow

```
POST /v1/messages with image block
        │
        ▼
┌───────────────────────────────────┐
│ Deserialize AnthropicRequest      │
│ • ContentBlock::Image recognized  │
│ • ImageSource::Base64 parsed      │
└───────────────────────────────────┘
        │
        ▼
┌───────────────────────────────────┐
│ to_chat_completion_request()      │
│ • Detect image blocks             │
│ • Call translate_content_block()  │
└───────────────────────────────────┘
        │
        ▼
┌───────────────────────────────────┐
│ translate_content_block()         │
│ • Image → ImageUrl                │
│ • Base64 → data URI               │
│ • Document → None (skip)          │
└───────────────────────────────────┘
        │
        ▼
┌───────────────────────────────────┐
│ Build OpenAI request              │
│ • MessageContent::Blocks          │
│ • ContentBlock::ImageUrl          │
└───────────────────────────────────┘
        │
        ▼
   Send to GitHub Copilot
```

### Key Design Decisions

| Decision | Rationale |
|----------|-----------|
| **Translate to image_url format** | OpenAI standard; LiteLLM proven working |
| **Data URI for base64** | Standard format: `data:{media_type};base64,{data}` |
| **Skip documents with warning** | No OpenAI equivalent; graceful degradation |
| **Accept cache_control** | API compatibility; silently ignored |
| **Tagged enums for sources** | Type safety; clear deserialization |
| **Optional cache_control** | Most messages don't use it; backward compatible |

---

## Dependencies

### New Dependencies

None required. Uses existing:
- `serde::{Serialize, Deserialize}` for JSON handling
- `tracing::warn!` for logging document skip warnings

### Sequencing Constraints

1. **Epic 1** (Anthropic types) must complete first
2. **Epic 2** (OpenAI types) can proceed in parallel with Epic 1
3. **Epic 3** (Translation) depends on Epics 1 and 2
4. **Epic 4** (Testing) depends on all previous epics

---

## Impact Analysis

### Files Modified

| File Path | Changes |
|-----------|---------|
| `src/anthropic/types.rs` | Add `ImageSource`, `DocumentSource`, `CacheControl`; extend `ContentBlock`; add `translate_content_block()` |
| `src/copilot/types.rs` | Add `ImageUrl` struct; extend `ContentBlock` with `ImageUrl` variant; update `as_text()` |

### Files Created

| File Path | Purpose |
|-----------|---------|
| `tests/unit/anthropic_image_tests.rs` | Unit tests for image/document deserialization and translation |
| `tests/integration/messages_multimodal_tests.rs` | Integration tests for image upload via `/v1/messages` |

---

## Risks and Mitigations

| # | Risk | Likelihood | Impact | Mitigation |
|---|------|------------|--------|------------|
| R1 | GitHub Copilot rejects image_url format | Low | High | ✅ Confirmed working in LiteLLM; same format |
| R2 | Large images cause memory issues | Low | Medium | User responsibility; same as OpenAI direct |
| R3 | Cache control breaks existing tests | Low | Low | Make optional; update affected tests |
| R4 | Document skip surprises users | Medium | Low | Log clear warning; document limitation |
| R5 | Deserialization fails for unknown source types | Low | Medium | Use tagged enums; serde handles gracefully |

---

## Implementation Plan

### Epic 1: Anthropic Content Block Types

**Goal:** Add image, document, and cache control support to Anthropic types.

**Prerequisites:** None (can start immediately)

**Status:** DONE

**Tasks:**

| Task ID | Type | Description | Files | Status |
|---------|------|-------------|-------|--------|
| E1-T1 | IMPL | Create `ImageSource` enum with `Base64` and `Url` variants | `src/anthropic/types.rs` | DONE |
| E1-T2 | IMPL | Create `DocumentSource` enum with `Base64`, `Text`, `Url` variants | `src/anthropic/types.rs` | DONE |
| E1-T3 | IMPL | Create `CacheControl` struct with `cache_type` and `ttl` | `src/anthropic/types.rs` | DONE |
| E1-T4 | IMPL | Add `Image` variant to `ContentBlock` enum | `src/anthropic/types.rs` | DONE |
| E1-T5 | IMPL | Add `Document` variant to `ContentBlock` enum | `src/anthropic/types.rs` | DONE |
| E1-T6 | IMPL | Add `cache_control: Option<CacheControl>` to existing variants | `src/anthropic/types.rs` | DONE |
| E1-T7 | IMPL | Update `extract_text()` to handle `Image` (return `"[Image]"`) | `src/anthropic/types.rs` | DONE |
| E1-T8 | IMPL | Update `extract_text()` to handle `Document` (return title or `"[Document]"`) | `src/anthropic/types.rs` | DONE |
| E1-T9 | TEST | Unit test: deserialize image block with base64 source | `tests/unit/anthropic_image_tests.rs` | DONE |
| E1-T10 | TEST | Unit test: deserialize image block with URL source | `tests/unit/anthropic_image_tests.rs` | DONE |
| E1-T11 | TEST | Unit test: deserialize document block | `tests/unit/anthropic_image_tests.rs` | DONE |
| E1-T12 | TEST | Unit test: deserialize cache_control on text block | `tests/unit/anthropic_image_tests.rs` | DONE |
| E1-T13 | TEST | Unit test: `extract_text()` handles image/document blocks | `tests/unit/anthropic_image_tests.rs` | DONE |

**Acceptance Criteria:**
- [x] All new types compile without errors
- [x] Deserialization works for image blocks (base64 and URL)
- [x] Deserialization works for document blocks
- [x] Cache control fields deserialize correctly
- [x] `extract_text()` produces sensible placeholders
- [x] All unit tests pass

**Code References:**

Example `ImageSource`:
```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum ImageSource {
    #[serde(rename = "base64")]
    Base64 {
        media_type: String,
        data: String,
    },
    #[serde(rename = "url")]
    Url {
        media_type: String,
        url: String,
    },
}
```

---

### Epic 2: OpenAI Multimodal Support

**Goal:** Add `image_url` content block support to OpenAI types.

**Prerequisites:** None (can run in parallel with Epic 1)

**Status:** Not Started

**Tasks:**

| Task ID | Type | Description | Files | Status |
|---------|------|-------------|-------|--------|
| E2-T1 | IMPL | Create `ImageUrl` struct with `url` and `detail` fields | `src/copilot/types.rs` | TODO |
| E2-T2 | IMPL | Add `ImageUrl` variant to `ContentBlock` enum | `src/copilot/types.rs` | TODO |
| E2-T3 | IMPL | Update `MessageContent::as_text()` to skip `ImageUrl` blocks | `src/copilot/types.rs` | TODO |
| E2-T4 | TEST | Unit test: serialize `ImageUrl` block to JSON | `tests/unit/copilot_types_tests.rs` | TODO |
| E2-T5 | TEST | Unit test: `as_text()` skips image blocks | `tests/unit/copilot_types_tests.rs` | TODO |

**Acceptance Criteria:**
- [ ] `ImageUrl` struct compiles and serializes correctly
- [ ] `ContentBlock::ImageUrl` variant works
- [ ] `as_text()` gracefully skips images
- [ ] JSON output matches OpenAI format
- [ ] All unit tests pass

**Code References:**

Example `ImageUrl`:
```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImageUrl {
    pub url: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}
```

---

### Epic 3: Translation Logic

**Goal:** Implement translation from Anthropic image blocks to OpenAI image_url format.

**Prerequisites:** Epic 1 (Anthropic types), Epic 2 (OpenAI types)

**Status:** Not Started

**Tasks:**

| Task ID | Type | Description | Files | Status |
|---------|------|-------------|-------|--------|
| E3-T1 | IMPL | Create `translate_content_block()` helper function | `src/anthropic/types.rs` | TODO |
| E3-T2 | IMPL | Handle `ContentBlock::Text` → `copilot::ContentBlock::Text` | `src/anthropic/types.rs` | TODO |
| E3-T3 | IMPL | Handle `ContentBlock::Image` → `copilot::ContentBlock::ImageUrl` | `src/anthropic/types.rs` | TODO |
| E3-T4 | IMPL | Convert `ImageSource::Base64` to data URI format | `src/anthropic/types.rs` | TODO |
| E3-T5 | IMPL | Pass through `ImageSource::Url` unchanged | `src/anthropic/types.rs` | TODO |
| E3-T6 | IMPL | Handle `ContentBlock::Document` → `None` with warning log | `src/anthropic/types.rs` | TODO |
| E3-T7 | IMPL | Update `to_chat_completion_request()` to detect multimodal messages | `src/anthropic/types.rs` | TODO |
| E3-T8 | IMPL | Build `MessageContent::Blocks` for multimodal messages | `src/anthropic/types.rs` | TODO |
| E3-T9 | TEST | Unit test: translate text block | `tests/unit/anthropic_image_tests.rs` | TODO |
| E3-T10 | TEST | Unit test: translate image with base64 → data URI | `tests/unit/anthropic_image_tests.rs` | TODO |
| E3-T11 | TEST | Unit test: translate image with URL → URL | `tests/unit/anthropic_image_tests.rs` | TODO |
| E3-T12 | TEST | Unit test: document block skipped with warning | `tests/unit/anthropic_image_tests.rs` | TODO |
| E3-T13 | TEST | Unit test: mixed content (text + image) | `tests/unit/anthropic_image_tests.rs` | TODO |
| E3-T14 | TEST | Unit test: full request translation with images | `tests/unit/anthropic_image_tests.rs` | TODO |

**Acceptance Criteria:**
- [ ] `translate_content_block()` handles all block types
- [ ] Base64 images converted to `data:{media_type};base64,{data}` format
- [ ] URL images passed through unchanged
- [ ] Document blocks skipped with `tracing::warn!`
- [ ] `to_chat_completion_request()` builds multimodal messages
- [ ] All unit tests pass

**Code References:**

Example translation:
```rust
fn translate_content_block(block: &ContentBlock) -> Option<crate::copilot::types::ContentBlock> {
    use crate::copilot::types;

    match block {
        ContentBlock::Text { text, .. } => {
            Some(types::ContentBlock::Text {
                text: text.clone(),
            })
        }
        ContentBlock::Image { source, .. } => {
            let url = match source {
                ImageSource::Base64 { media_type, data } => {
                    format!("data:{};base64,{}", media_type, data)
                }
                ImageSource::Url { url, .. } => url.clone(),
            };
            Some(types::ContentBlock::ImageUrl {
                image_url: types::ImageUrl {
                    url,
                    detail: None,
                },
            })
        }
        ContentBlock::Document { title, .. } => {
            tracing::warn!(
                title = title.as_deref(),
                "Document content blocks are not supported by OpenAI format; skipping"
            );
            None
        }
        _ => None,
    }
}
```

---

### Epic 4: Integration Testing

**Goal:** End-to-end testing with mock Copilot server and real adapter.

**Prerequisites:** Epic 3 (translation logic)

**Status:** Not Started

**Tasks:**

| Task ID | Type | Description | Files | Status |
|---------|------|-------------|-------|--------|
| E4-T1 | TEST | Create integration test with image block (base64) | `tests/integration/messages_multimodal_tests.rs` | TODO |
| E4-T2 | TEST | Create integration test with image block (URL) | `tests/integration/messages_multimodal_tests.rs` | TODO |
| E4-T3 | TEST | Create integration test with mixed content (text + image) | `tests/integration/messages_multimodal_tests.rs` | TODO |
| E4-T4 | TEST | Create integration test with document block (verify skip) | `tests/integration/messages_multimodal_tests.rs` | TODO |
| E4-T5 | TEST | Verify mock Copilot receives correct OpenAI format | `tests/integration/messages_multimodal_tests.rs` | TODO |
| E4-T6 | TEST | Verify response is valid Anthropic format | `tests/integration/messages_multimodal_tests.rs` | TODO |
| E4-T7 | TEST | Test with cache_control (verify accepted) | `tests/integration/messages_multimodal_tests.rs` | TODO |

**Acceptance Criteria:**
- [ ] Image blocks deserialize without 422 errors
- [ ] Translated requests match OpenAI multimodal format
- [ ] Responses are valid Anthropic format
- [ ] Document skip warning logged
- [ ] Cache control accepted but not forwarded
- [ ] All integration tests pass

---

### Epic 5: Manual E2E Testing

**Goal:** Verify with real Claude Code and GitHub Copilot.

**Prerequisites:** Epic 4 (integration tests passing)

**Status:** Not Started

**Tasks:**

| Task ID | Type | Description | Status |
|---------|------|-------------|--------|
| E5-T1 | MANUAL | Build adapter: `cargo build --release` | TODO |
| E5-T2 | MANUAL | Start adapter: `./target/release/copilot-adapter start` | TODO |
| E5-T3 | MANUAL | Configure Claude Code to use adapter endpoint | TODO |
| E5-T4 | MANUAL | Upload image via Claude Code | TODO |
| E5-T5 | MANUAL | Verify no 422 error in logs | TODO |
| E5-T6 | MANUAL | Verify image reaches model and response includes analysis | TODO |
| E5-T7 | DOC | Update README with vision support information | TODO |

**Acceptance Criteria:**
- [ ] Claude Code image upload succeeds
- [ ] No deserialization errors in adapter logs
- [ ] Model receives and processes image
- [ ] README documents vision support

---

## Verification Steps

### Unit Test Validation

```bash
# Run unit tests
cargo test --test unit

# Expected output:
# test anthropic_image_tests::deserialize_image_base64 ... ok
# test anthropic_image_tests::deserialize_image_url ... ok
# test anthropic_image_tests::translate_image_to_data_uri ... ok
# test anthropic_image_tests::translate_mixed_content ... ok
# test anthropic_image_tests::skip_document_with_warning ... ok
```

### Integration Test Validation

```bash
# Run integration tests
cargo test --test integration

# Expected output:
# test messages_multimodal_tests::image_upload_base64 ... ok
# test messages_multimodal_tests::image_upload_url ... ok
# test messages_multimodal_tests::mixed_text_and_image ... ok
# test messages_multimodal_tests::document_block_skipped ... ok
```

### Manual Curl Test

```bash
# Test image upload endpoint
curl -X POST http://localhost:6767/v1/messages \
  -H "Content-Type: application/json" \
  -d '{
    "model": "gpt-4o",
    "max_tokens": 1024,
    "messages": [{
      "role": "user",
      "content": [{
        "type": "text",
        "text": "What is in this image?"
      }, {
        "type": "image",
        "source": {
          "type": "base64",
          "media_type": "image/jpeg",
          "data": "/9j/4AAQSkZJRg..."
        }
      }]
    }]
  }'

# Expected: Valid Anthropic response, no 422 error
```

---

## Rollout Plan

### Phase 1: Code Complete
- All epics 1-3 tasks complete
- Unit tests passing
- Code review complete

### Phase 2: Integration Validation
- Epic 4 tasks complete
- Integration tests passing
- No regressions in existing tests

### Phase 3: Manual Validation
- Epic 5 tasks complete
- Real image upload working
- Documentation updated

### Phase 4: Release
- Merge to main branch
- Update CHANGELOG.md
- Tag new version
- Update README with vision support details

---

## Success Metrics

| Metric | Target | Measurement |
|--------|--------|-------------|
| Image upload success rate | 100% | Manual testing with Claude Code |
| Deserialization errors | 0 | Adapter logs during testing |
| Unit test coverage | >90% | `cargo tarpaulin` |
| Integration test coverage | All scenarios | Test pass rate |
| Performance impact (text-only) | <1ms overhead | Benchmark comparison |
| Backward compatibility | 100% | Existing tests still pass |

---

## Future Enhancements

These are explicitly out of scope for this implementation but may be considered later:

1. **Document extraction** — Extract text from text-type documents for limited support
2. **Extended block types** — Add `thinking`, `search_result`, etc. for full API parity
3. **Streaming images** — Support image blocks in streaming responses (if needed)
4. **Image optimization** — Compress large images before sending to Copilot
5. **Cache control translation** — If OpenAI adds caching support
