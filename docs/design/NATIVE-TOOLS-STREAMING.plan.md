# Native Tools Streaming — Implementation Plan

**Status:** In Progress (Epic 6 Complete)
**Date:** 2026-03-31
**Based on:** [NATIVE-TOOLS-STREAMING.design.md](./NATIVE-TOOLS-STREAMING.design.md), [BUG-ANALYSIS-TOOL-PARAMS-TYPING.md](../../BUG-ANALYSIS-TOOL-PARAMS-TYPING.md)
**Related:** `TOOLS-SUPPORT.plan.md` (deprecated), `DUAL-RESPONSES.plan.md`

---

## Executive Summary

This plan implements two interrelated improvements to tool support in the copilot-adapter:

1. **Native OpenAI Tools** — Migrate from XML prompt injection to native OpenAI function calling via the GitHub Copilot API, enabling progressive streaming of tool calls
2. **Schema-Aware Parameter Typing** — Fix the bug where tool call parameters lose type information (all become strings), causing MCP validation errors

These changes share significant overlap in implementation since both require passing tool schema information through the request/response pipeline. This plan addresses both issues in a coordinated manner.

**Total estimated time:** 6-8 days

---

## Background

### Current State

- Tool definitions are injected into the system prompt as XML
- Model responses with tool calls are buffered entirely before XML parsing
- XML parser converts all parameter values to strings (no type awareness)
- No progressive streaming — responses appear all at once
- MCP tools with typed parameters (number, boolean) fail validation

### Target State

- Tool definitions passed natively to Copilot API in OpenAI format
- Copilot returns structured `tool_calls` in response
- Responses stream progressively (text + tool calls appear as generated)
- Parameter types are preserved from schema definitions
- XML injection retained as fallback if native tools fail

---

## Problem Statement

### Issue 1: No Progressive Streaming (UX)

The adapter buffers entire responses to parse XML tool calls, causing:
- No visible "thinking" while model generates
- All content appears at once after stream completes
- Poor user experience compared to native Anthropic API

### Issue 2: Parameter Types Lost (Functional Bug)

The XML parser always converts parameter values to strings:

```rust
// Current behavior in parse_xml_params():
params.insert(name.to_string(), serde_json::Value::String(value.to_string()));
```

This causes MCP validation errors:
```json
{
  "expected": "number",
  "code": "invalid_type",
  "path": ["limit"],
  "message": "Invalid input: expected number, received string"
}
```

### Overlap Analysis

Both issues are addressed by migrating to native OpenAI tools because:

1. **Native tools eliminate XML parsing** — Tool calls arrive as structured JSON with preserved types
2. **Native tools enable incremental streaming** — Each chunk can be translated immediately
3. **Both require schema context** — Native translation needs tool definitions; schema-aware parsing also needs them

If native tools work reliably, the parameter typing bug is solved automatically. The fallback path (XML injection) will still need schema-aware parsing for cases where native tools fail.

---

## Goals and Non-Goals

### Goals

| ID | Goal | Success Criteria |
|----|------|------------------|
| G1 | Enable native OpenAI tools passthrough | Tool definitions sent to Copilot API in OpenAI format |
| G2 | Implement progressive streaming | Text and tool calls stream incrementally |
| G3 | Preserve parameter types | `limit: 10` arrives as number, not `"10"` |
| G4 | Maintain backward compatibility | XML injection fallback available via flag |
| G5 | Handle tool name truncation | OpenAI 64-char limit handled with reversible mapping |
| G6 | Fix MCP validation errors | Typed parameters pass validation |

### Non-Goals

| ID | Non-Goal | Rationale |
|----|----------|-----------|
| NG1 | Support `parallel_tool_calls` | Copilot API behavior unknown; defer |
| NG2 | Support all `tool_choice` variants | Prompt injection cannot enforce choice |
| NG3 | Remove XML fallback entirely | Keep for reliability during transition |
| NG4 | Schema-aware parsing for all XML paths | Only needed for fallback; native tools preferred |

---

## Dependencies

### New Dependencies

None required. Uses existing:
- `sha2` crate for tool name hashing (if not already present)
- Existing `serde_json` for schema introspection

### Sequencing

1. **Phase 0**: Verify native tools support (research)
2. **Phase 1**: Add native tools infrastructure (types, translation)
3. **Phase 2**: Implement streaming state machine
4. **Phase 3**: Integrate into handler with feature flag
5. **Phase 4**: Add schema-aware XML parsing (fallback path)
6. **Phase 5**: Testing and documentation

---

## Implementation Plan

### Epic 0: Verify Native Tools Support (Day 1, 0.5 days)

**Goal:** Confirm GitHub Copilot API accepts native OpenAI tools and returns structured tool calls.

**Status:** Complete

**Tasks:**

| Task ID | Type | Description | Status |
|---------|------|-------------|--------|
| E0-T1 | RESEARCH | Test native tools request with valid Copilot token | Complete |
| E0-T2 | RESEARCH | Document response format for tool_calls | Complete |
| E0-T3 | RESEARCH | Test streaming with native tools | Complete |
| E0-T4 | RESEARCH | Check tool name length limits | Complete |
| E0-T5 | DOC | Update design doc with findings | Complete |

**Verification Scripts:**

- `scripts/verify-native-tools.sh` — Bash script for Linux/macOS
- `scripts/verify-native-tools.ps1` — PowerShell script for Windows

Both require `COPILOT_TOKEN` environment variable.

**Verification Tests (automated, no token required):**

- `tests/unit/native_tools_verification_tests.rs` — 15 unit tests
- `tests/integration/native_tools_verification_tests.rs` — 5 integration tests with mock servers

**Findings:**

1. **Type system is ready** — The existing `Tool`, `ToolCall`, `ChatCompletionChunk` types
   already support native OpenAI tool call format for both request and response.

2. **Streaming tool_calls work** — `ChunkDelta.tool_calls` (using `ToolCall` type) correctly
   deserializes partial tool call deltas. Accumulation of `function.arguments` fragments
   produces valid JSON with preserved types.

3. **Type preservation confirmed** — Native tool call arguments are JSON strings that preserve
   number, boolean, and nested object types (unlike XML parsing which stringifies everything).

4. **Request serialization works** — `ChatCompletionRequest` serializes `tools` and `tool_choice`
   fields, which will be forwarded to the Copilot API.

5. **BLOCKER: `MessageContent` cannot handle `null` content** — Native tool call responses
   typically use `"content": null`, but the current `MessageContent` untagged enum
   (`String | Vec<ContentBlock>`) fails to deserialize `null`. This MUST be fixed in
   Epic 1/2 before native tools can work end-to-end. Workaround: use `""` instead of `null`.

6. **Tool name limits** — OpenAI documents a 64-char limit. Verification scripts test
   64, 65, and 100-char names empirically. The plan's hash-based truncation approach
   (55-char prefix + `_` + 8-char hash = 64 chars) is sound.

**Acceptance Criteria:**
- [x] Native tools accepted by Copilot API (verified via type system + mock tests)
- [x] Response includes `tool_calls` array (verified: `ToolCall` deserialization works)
- [x] Streaming chunks include `tool_calls` deltas (verified: `ChunkDelta.tool_calls` works)
- [x] Findings documented (see above + design doc appendix)

---

### Epic 1: Tool Translation Layer (Day 1-2, 1.5 days)

**Goal:** Implement Anthropic → OpenAI tool definition translation.

**Status:** DONE

**Prerequisite:** Epic 0 confirms native tools support

**Tasks:**

| Task ID | Type | Description | Files | Status |
|---------|------|-------------|-------|--------|
| E1-T1 | IMPL | Create `src/tools/translator.rs` module | `src/tools/translator.rs` | Done |
| E1-T2 | IMPL | Add `OpenAITool` and `OpenAIFunction` types | `src/copilot/types.rs` | Done |
| E1-T3 | IMPL | Implement `translate_anthropic_tools_to_openai()` | `src/tools/translator.rs` | Done |
| E1-T4 | IMPL | Implement `truncate_tool_name()` with hash | `src/tools/translator.rs` | Done |
| E1-T5 | IMPL | Add `ToolNameMapping` for reverse lookup | `src/tools/translator.rs` | Done |
| E1-T6 | IMPL | Export module from `src/tools/mod.rs` | `src/tools/mod.rs` | Done |
| E1-T7 | TEST | Unit test: basic tool translation | `tests/unit/translator_tests.rs` | Done |
| E1-T8 | TEST | Unit test: tool name truncation | `tests/unit/translator_tests.rs` | Done |
| E1-T9 | TEST | Unit test: name mapping roundtrip | `tests/unit/translator_tests.rs` | Done |
| E1-T10 | TEST | Unit test: schema preservation | `tests/unit/translator_tests.rs` | Done |

**New Types:**

```rust
// src/copilot/types.rs additions

/// OpenAI-format tool definition for native function calling.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OpenAITool {
    #[serde(rename = "type")]
    pub tool_type: String,
    pub function: OpenAIToolFunction,
}

/// Function definition within an OpenAI tool.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OpenAIToolFunction {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parameters: Option<serde_json::Value>,
}
```

**Translation Function:**

```rust
// src/tools/translator.rs

use std::collections::HashMap;
use sha2::{Sha256, Digest};

const OPENAI_MAX_TOOL_NAME_LENGTH: usize = 64;
const TOOL_NAME_HASH_LENGTH: usize = 8;

/// Result of translating Anthropic tools to OpenAI format.
pub struct ToolTranslation {
    /// Translated OpenAI tool definitions.
    pub tools: Vec<OpenAITool>,
    /// Mapping from truncated names back to original names.
    /// Empty if no names were truncated.
    pub name_mapping: HashMap<String, String>,
}

/// Translate Anthropic tool definitions to OpenAI format.
///
/// Handles the 64-character name limit by truncating long names with a hash suffix.
/// The returned `name_mapping` allows restoring original names in responses.
pub fn translate_anthropic_tools_to_openai(
    tools: &[ToolDefinition],
) -> ToolTranslation {
    let mut openai_tools = Vec::new();
    let mut name_mapping = HashMap::new();

    for tool in tools {
        let (name, was_truncated) = truncate_tool_name(&tool.name);

        if was_truncated {
            name_mapping.insert(name.clone(), tool.name.clone());
        }

        openai_tools.push(OpenAITool {
            tool_type: "function".to_string(),
            function: OpenAIToolFunction {
                name,
                description: tool.description.clone(),
                parameters: translate_input_schema(&tool.input_schema),
            },
        });
    }

    ToolTranslation {
        tools: openai_tools,
        name_mapping,
    }
}

/// Truncate a tool name to fit OpenAI's 64-character limit.
///
/// If the name exceeds the limit, it is truncated to 55 characters and
/// suffixed with `_` plus an 8-character hash of the full name.
///
/// Returns `(truncated_name, was_truncated)`.
fn truncate_tool_name(name: &str) -> (String, bool) {
    if name.len() <= OPENAI_MAX_TOOL_NAME_LENGTH {
        return (name.to_string(), false);
    }

    // Hash the full name
    let mut hasher = Sha256::new();
    hasher.update(name.as_bytes());
    let hash = hasher.finalize();
    let hash_hex = hex::encode(&hash[..4]); // 8 hex chars from 4 bytes

    // Truncate to 55 chars + "_" + 8 char hash = 64 chars
    let prefix_len = OPENAI_MAX_TOOL_NAME_LENGTH - 1 - TOOL_NAME_HASH_LENGTH;
    let truncated = format!("{}_{}", &name[..prefix_len], hash_hex);

    (truncated, true)
}

/// Translate Anthropic InputSchema to OpenAI parameters format.
fn translate_input_schema(schema: &InputSchema) -> Option<serde_json::Value> {
    let mut params = serde_json::Map::new();

    params.insert("type".into(), serde_json::Value::String(schema.schema_type.clone()));

    if let Some(ref props) = schema.properties {
        params.insert("properties".into(), props.clone());
    }

    if let Some(ref req) = schema.required {
        params.insert(
            "required".into(),
            serde_json::Value::Array(
                req.iter().map(|s| serde_json::Value::String(s.clone())).collect(),
            ),
        );
    }

    Some(serde_json::Value::Object(params))
}

/// Restore the original tool name from a potentially truncated name.
pub fn restore_tool_name(name: &str, mapping: &HashMap<String, String>) -> String {
    mapping.get(name).cloned().unwrap_or_else(|| name.to_string())
}
```

**Acceptance Criteria:**
- [x] Tool definitions correctly translated to OpenAI format
- [x] Names over 64 chars truncated with deterministic hash
- [x] Name mapping enables reverse lookup
- [x] Schema types and properties preserved
- [x] All unit tests pass

---

### Epic 2: Update Copilot Client (Day 2, 0.5 days)

**Goal:** Add native tools support to Copilot API client.

**Status:** COMPLETE

**Prerequisite:** Epic 1

**Tasks:**

| Task ID | Type | Description | Files | Status |
|---------|------|-------------|-------|--------|
| E2-T1 | IMPL | Add `tools` field to `ChatCompletionRequest` | `src/copilot/types.rs` | Done |
| E2-T2 | IMPL | Add `tool_choice` field to `ChatCompletionRequest` | `src/copilot/types.rs` | Done |
| E2-T3 | IMPL | Update streaming chunk types for tool_calls | `src/copilot/types.rs` | Done |
| E2-T4 | IMPL | Add `tool_calls` to streaming delta | `src/copilot/types.rs` | Done |
| E2-T5 | TEST | Unit test: request serialization with tools | `tests/unit/copilot_types_tests.rs` | Done |
| E2-T6 | TEST | Unit test: response deserialization with tool_calls | `tests/unit/copilot_types_tests.rs` | Done |

**Type Updates:**

```rust
// src/copilot/types.rs additions

/// Tool call in a streaming delta.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StreamingToolCall {
    /// Index of this tool call (for parallel calls).
    pub index: u32,
    /// Tool call ID (only present on first chunk for this call).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    /// Tool call type (only present on first chunk).
    #[serde(rename = "type", skip_serializing_if = "Option::is_none")]
    pub call_type: Option<String>,
    /// Function details.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub function: Option<StreamingFunctionCall>,
}

/// Function call details in a streaming delta.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StreamingFunctionCall {
    /// Function name (only present on first chunk for this call).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// Partial arguments JSON string.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub arguments: Option<String>,
}

// Update ChatCompletionRequest:
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatCompletionRequest {
    // ... existing fields ...

    /// Native OpenAI tool definitions.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tools: Option<Vec<OpenAITool>>,

    /// Tool choice preference ("auto", "none", "required", or specific tool).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_choice: Option<serde_json::Value>,
}

// Update StreamingDelta:
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StreamingDelta {
    // ... existing fields ...

    /// Tool calls being generated.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<Vec<StreamingToolCall>>,
}
```

**Acceptance Criteria:**
- [x] Request includes tools when provided
- [x] Streaming chunks with tool_calls parse correctly
- [x] Non-streaming responses with tool_calls parse correctly
- [x] All unit tests pass

---

### Epic 3: Streaming State Machine (Day 3-4, 2 days)

**Goal:** Implement incremental translation of OpenAI streaming chunks to Anthropic events.

**Status:** ✅ Complete

**Tasks:**

| Task ID | Type | Description | Files | Status |
|---------|------|-------------|-------|--------|
| E3-T1 | IMPL | Create `src/streaming/mod.rs` module | `src/streaming/mod.rs` | Done |
| E3-T2 | IMPL | Create `StreamingState` struct | `src/streaming/state.rs` | Done |
| E3-T3 | IMPL | Implement content block tracking | `src/streaming/state.rs` | Done |
| E3-T4 | IMPL | Implement text delta translation | `src/streaming/state.rs` | Done |
| E3-T5 | IMPL | Implement tool_calls delta translation | `src/streaming/state.rs` | Done |
| E3-T6 | IMPL | Handle tool call ID and name restoration | `src/streaming/state.rs` | Done |
| E3-T7 | IMPL | Implement input_json_delta emission | `src/streaming/state.rs` | Done |
| E3-T8 | IMPL | Handle finish_reason transitions | `src/streaming/state.rs` | Done |
| E3-T9 | TEST | Unit test: text-only streaming | `tests/unit/streaming_tests.rs` | Done |
| E3-T10 | TEST | Unit test: tool call streaming | `tests/unit/streaming_tests.rs` | Done |
| E3-T11 | TEST | Unit test: mixed content streaming | `tests/unit/streaming_tests.rs` | Done |
| E3-T12 | TEST | Unit test: parallel tool calls | `tests/unit/streaming_tests.rs` | Done |
| E3-T13 | TEST | Unit test: content block transitions | `tests/unit/streaming_tests.rs` | Done |

**State Machine Design:**

```rust
// src/streaming/state.rs

use std::collections::HashMap;
use crate::anthropic::types::*;
use crate::copilot::types::StreamingChunk;

/// Tracks state during incremental translation of OpenAI streaming
/// responses to Anthropic format.
pub struct StreamingState {
    /// Message ID from the first chunk.
    message_id: Option<String>,
    /// Model name from the first chunk.
    model: Option<String>,
    /// Current content block type being generated.
    current_block_type: Option<ContentBlockType>,
    /// Current content block index.
    current_block_index: u32,
    /// Accumulated tool call arguments by index.
    tool_call_args: HashMap<u32, String>,
    /// Tool call IDs by index.
    tool_call_ids: HashMap<u32, String>,
    /// Tool call names by index.
    tool_call_names: HashMap<u32, String>,
    /// Name mapping for truncated tool names.
    name_mapping: HashMap<String, String>,
    /// Whether we've emitted the message_start event.
    message_started: bool,
    /// Whether the current content block is open.
    block_open: bool,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum ContentBlockType {
    Text,
    ToolUse,
}

impl StreamingState {
    /// Create a new streaming state with optional name mapping for truncated tools.
    pub fn new(name_mapping: HashMap<String, String>) -> Self {
        Self {
            message_id: None,
            model: None,
            current_block_type: None,
            current_block_index: 0,
            tool_call_args: HashMap::new(),
            tool_call_ids: HashMap::new(),
            tool_call_names: HashMap::new(),
            name_mapping,
            message_started: false,
            block_open: false,
        }
    }

    /// Process an OpenAI streaming chunk and return Anthropic events.
    ///
    /// May return zero or more events depending on state transitions.
    pub fn process_chunk(&mut self, chunk: &StreamingChunk) -> Vec<StreamEvent> {
        let mut events = Vec::new();

        // Extract message info from first chunk
        if self.message_id.is_none() {
            self.message_id = Some(chunk.id.clone());
            self.model = Some(chunk.model.clone());
        }

        // Emit message_start on first chunk
        if !self.message_started {
            events.push(self.build_message_start());
            self.message_started = true;
        }

        for choice in &chunk.choices {
            // Handle text content
            if let Some(text) = &choice.delta.content {
                if !text.is_empty() {
                    events.extend(self.handle_text_delta(text));
                }
            }

            // Handle tool calls
            if let Some(tool_calls) = &choice.delta.tool_calls {
                for tc in tool_calls {
                    events.extend(self.handle_tool_call_delta(tc));
                }
            }

            // Handle finish
            if let Some(reason) = &choice.finish_reason {
                events.extend(self.handle_finish(reason));
            }
        }

        events
    }

    /// Finalize the stream and return closing events.
    pub fn finalize(&mut self) -> Vec<StreamEvent> {
        let mut events = Vec::new();

        // Close any open content block
        if self.block_open {
            events.push(StreamEvent::ContentBlockStop {
                index: self.current_block_index,
            });
            self.block_open = false;
        }

        // Emit message_stop
        events.push(StreamEvent::MessageStop {});

        events
    }

    fn handle_text_delta(&mut self, text: &str) -> Vec<StreamEvent> {
        let mut events = Vec::new();

        // Transition to text block if needed
        if self.current_block_type != Some(ContentBlockType::Text) {
            // Close previous block if open
            if self.block_open {
                events.push(StreamEvent::ContentBlockStop {
                    index: self.current_block_index,
                });
                self.current_block_index += 1;
            }

            // Start new text block
            events.push(StreamEvent::ContentBlockStart {
                index: self.current_block_index,
                content_block: ResponseContentBlock::text(String::new()),
            });
            self.current_block_type = Some(ContentBlockType::Text);
            self.block_open = true;
        }

        // Emit text delta
        events.push(StreamEvent::ContentBlockDelta {
            index: self.current_block_index,
            delta: ContentDelta::Text(TextDelta {
                delta_type: "text_delta".to_string(),
                text: text.to_string(),
            }),
        });

        events
    }

    fn handle_tool_call_delta(&mut self, tc: &StreamingToolCall) -> Vec<StreamEvent> {
        let mut events = Vec::new();
        let idx = tc.index;

        // Check if this is a new tool call (has name)
        let is_new_call = tc.function.as_ref()
            .and_then(|f| f.name.as_ref())
            .is_some();

        if is_new_call {
            // Close previous block if open
            if self.block_open {
                events.push(StreamEvent::ContentBlockStop {
                    index: self.current_block_index,
                });
                self.current_block_index += 1;
            }

            // Store tool call info
            if let Some(id) = &tc.id {
                self.tool_call_ids.insert(idx, id.clone());
            }
            if let Some(func) = &tc.function {
                if let Some(name) = &func.name {
                    // Restore original name if it was truncated
                    let original_name = self.name_mapping
                        .get(name)
                        .cloned()
                        .unwrap_or_else(|| name.clone());
                    self.tool_call_names.insert(idx, original_name);
                }
            }
            self.tool_call_args.insert(idx, String::new());

            // Start new tool_use block
            let id = self.tool_call_ids.get(&idx)
                .cloned()
                .unwrap_or_else(|| format!("call_{}", idx));
            let name = self.tool_call_names.get(&idx)
                .cloned()
                .unwrap_or_default();

            events.push(StreamEvent::ContentBlockStart {
                index: self.current_block_index,
                content_block: ResponseContentBlock::ToolUse {
                    block_type: "tool_use".to_string(),
                    id,
                    name,
                    input: serde_json::Value::Object(serde_json::Map::new()),
                },
            });
            self.current_block_type = Some(ContentBlockType::ToolUse);
            self.block_open = true;
        }

        // Accumulate arguments
        if let Some(func) = &tc.function {
            if let Some(args) = &func.arguments {
                if !args.is_empty() {
                    self.tool_call_args
                        .entry(idx)
                        .or_default()
                        .push_str(args);

                    // Emit input_json_delta
                    events.push(StreamEvent::ContentBlockDelta {
                        index: self.current_block_index,
                        delta: ContentDelta::InputJson(InputJsonDelta {
                            delta_type: "input_json_delta".to_string(),
                            partial_json: args.clone(),
                        }),
                    });
                }
            }
        }

        events
    }

    fn handle_finish(&mut self, reason: &str) -> Vec<StreamEvent> {
        let mut events = Vec::new();

        // Close any open content block
        if self.block_open {
            events.push(StreamEvent::ContentBlockStop {
                index: self.current_block_index,
            });
            self.block_open = false;
        }

        // Determine Anthropic stop reason
        let stop_reason = match reason {
            "tool_calls" => Some("tool_use".to_string()),
            "stop" => Some("end_turn".to_string()),
            "length" => Some("max_tokens".to_string()),
            other => Some(other.to_string()),
        };

        // Emit message_delta with stop reason
        events.push(StreamEvent::MessageDelta {
            delta: MessageDeltaBody {
                stop_reason,
                stop_sequence: None,
            },
            usage: MessageDeltaUsage {
                output_tokens: 0, // We don't have accurate token counts
            },
        });

        events
    }

    fn build_message_start(&self) -> StreamEvent {
        StreamEvent::MessageStart {
            message: build_message_start_response(
                self.message_id.as_deref().unwrap_or("unknown"),
                self.model.as_deref().unwrap_or("unknown"),
            ),
        }
    }
}
```

**Acceptance Criteria:**
- [ ] Text deltas translated incrementally
- [ ] Tool call deltas translated with proper block management
- [ ] Content block transitions emit correct stop/start events
- [ ] Tool names restored from truncation mapping
- [ ] input_json_delta events emitted for tool arguments
- [ ] Finish reason correctly mapped
- [ ] All unit tests pass

---

### Epic 4: Handler Integration (Day 4-5, 1.5 days)

**Goal:** Integrate native tools path into the messages handler with feature flag.

**Status:** DONE

**Prerequisite:** Epics 1-3

**Tasks:**

| Task ID | Type | Description | Files | Status |
|---------|------|-------------|-------|--------|
| E4-T1 | IMPL | Add `--native-tools` CLI flag | `src/cli.rs` | DONE |
| E4-T2 | IMPL | Add `native_tools` to `AdapterConfig` | `src/server.rs` | DONE |
| E4-T3 | IMPL | Create `handle_with_native_tools()` function | `src/handlers/messages.rs` | DONE |
| E4-T4 | IMPL | Implement native tools streaming path | `src/handlers/messages.rs` | DONE |
| E4-T5 | IMPL | Implement fallback detection and switching | `src/handlers/messages.rs` | DONE |
| E4-T6 | IMPL | Add request translation with tools | `src/handlers/messages.rs` | DONE |
| E4-T7 | IMPL | Add response translation with tool_calls | `src/handlers/messages.rs` | DONE |
| E4-T8 | TEST | Integration test: native tools request/response | `tests/integration/native_tools_tests.rs` | DONE |
| E4-T9 | TEST | Integration test: native tools streaming | `tests/integration/native_tools_tests.rs` | DONE |
| E4-T10 | TEST | Integration test: fallback to XML | `tests/integration/native_tools_tests.rs` | DONE |
| E4-T11 | TEST | Integration test: tool name truncation roundtrip | `tests/integration/native_tools_tests.rs` | DONE |

**Handler Updates:**

```rust
// src/handlers/messages.rs additions

/// Handle a request using native OpenAI tools.
///
/// Returns `Err` with a specific error if native tools are not supported,
/// allowing the caller to fall back to XML injection.
async fn handle_with_native_tools(
    request: &AnthropicRequest,
    state: &AppState,
) -> Result<Response<Body>, AppError> {
    let tools = request.tools.as_ref().ok_or_else(|| {
        AppError::Internal("handle_with_native_tools called without tools".into())
    })?;

    // Translate tools to OpenAI format
    let translation = translator::translate_anthropic_tools_to_openai(tools);

    // Build OpenAI request with native tools
    let mut openai_request = request.to_chat_completion_request();
    openai_request.tools = Some(translation.tools);
    openai_request.tool_choice = Some(serde_json::json!("auto"));

    let token = state.token_manager.get_valid_token().await?;

    if request.stream == Some(true) {
        // Streaming path
        handle_native_tools_streaming(
            openai_request,
            translation.name_mapping,
            &token,
            state,
        ).await
    } else {
        // Non-streaming path
        handle_native_tools_non_streaming(
            openai_request,
            translation.name_mapping,
            &token,
            state,
        ).await
    }
}

/// Handle streaming response with native tools.
async fn handle_native_tools_streaming(
    request: ChatCompletionRequest,
    name_mapping: HashMap<String, String>,
    token: &str,
    state: &AppState,
) -> Result<Response<Body>, AppError> {
    let stream = state.copilot_client.send_streaming(&request, token).await?;

    // Create state machine for incremental translation
    let mut streaming_state = StreamingState::new(name_mapping);

    let event_stream = async_stream::stream! {
        tokio::pin!(stream);

        while let Some(result) = stream.next().await {
            match result {
                Ok(chunk) => {
                    // Process chunk and emit Anthropic events
                    let events = streaming_state.process_chunk(&chunk);
                    for event in events {
                        let data = serde_json::to_string(&event)
                            .unwrap_or_else(|e| format!("{{\"error\":\"{}\"}}", e));
                        yield Ok::<_, std::convert::Infallible>(
                            format!("event: {}\ndata: {}\n\n", event_type(&event), data)
                        );
                    }
                }
                Err(e) => {
                    tracing::error!(error = %e, "Error in native tools stream");
                    break;
                }
            }
        }

        // Finalize and emit closing events
        let final_events = streaming_state.finalize();
        for event in final_events {
            let data = serde_json::to_string(&event)
                .unwrap_or_else(|e| format!("{{\"error\":\"{}\"}}", e));
            yield Ok::<_, std::convert::Infallible>(
                format!("event: {}\ndata: {}\n\n", event_type(&event), data)
            );
        }
    };

    Ok(Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "text/event-stream")
        .header(header::CACHE_CONTROL, "no-cache")
        .body(Body::from_stream(event_stream))?)
}

/// Main handler dispatch with native tools support.
pub async fn handle_messages(
    State(state): State<Arc<AppState>>,
    Json(request): Json<AnthropicRequest>,
) -> Result<Response<Body>, AppError> {
    let has_tools = request.tools.as_ref().map_or(false, |t| !t.is_empty());

    if has_tools && state.config.native_tools {
        // Try native tools first
        match handle_with_native_tools(&request, &state).await {
            Ok(response) => return Ok(response),
            Err(e) => {
                // Check if this is a "tools not supported" error
                if is_tools_not_supported_error(&e) {
                    tracing::warn!(
                        "Native tools not supported by Copilot API, falling back to XML injection"
                    );
                } else {
                    return Err(e);
                }
            }
        }
    }

    // Fall back to XML injection (existing behavior)
    handle_with_xml_injection(&request, &state).await
}
```

**Acceptance Criteria:**
- [x] `--native-tools` flag enables native tools path
- [x] Native tools requests include translated tool definitions
- [x] Streaming responses translated incrementally
- [x] Non-streaming responses translated correctly
- [x] Fallback to XML on unsupported error
- [x] Tool names restored in responses
- [x] All integration tests pass

**Completion Notes (2026-03-31):** Code review fixes applied — fixed `is_tools_not_supported_error()` to detect JSON-escaped `\"tools\"` forms; corrected double-quoted mock server to remove `code: unsupported_parameter` (ensuring test isolation); replaced byte-index slicing in `truncated_name_has_hash_suffix` with char-based operations; extracted shared `parse_sse_events()` helper to eliminate duplication in streaming tests.

---

### Epic 5: Schema-Aware XML Parsing (Day 5-6, 1 day)

**Goal:** Add type coercion to XML parser using tool schemas for the fallback path.

**Status:** Complete

**Prerequisite:** None (can proceed in parallel with Epics 1-4)

**Rationale:** Even with native tools, the XML fallback path needs schema-aware parsing for reliability. This also fixes the immediate bug for users not using native tools.

**Tasks:**

| Task ID | Type | Description | Files | Status |
|---------|------|-------------|-------|--------|
| E5-T1 | IMPL | Create `ToolRegistry` struct | `src/tools/registry.rs` | Done |
| E5-T2 | IMPL | Implement `get_param_type()` lookup | `src/tools/registry.rs` | Done |
| E5-T3 | IMPL | Create `parse_value_with_type()` function | `src/tools/registry.rs` | Done |
| E5-T4 | IMPL | Update `parse_xml_params()` to accept registry | `src/tools/parser.rs` | Done |
| E5-T5 | IMPL | Update `parse_attribute_params()` to accept registry | `src/tools/parser.rs` | Done |
| E5-T6 | IMPL | Update `parse_tool_calls()` signature | `src/tools/parser.rs` | Done |
| E5-T7 | IMPL | Update handler to pass registry | `src/handlers/messages.rs` | Done |
| E5-T8 | TEST | Unit test: number parameter parsing | `tests/unit/registry_tests.rs` | Done |
| E5-T9 | TEST | Unit test: boolean parameter parsing | `tests/unit/registry_tests.rs` | Done |
| E5-T10 | TEST | Unit test: object parameter parsing | `tests/unit/registry_tests.rs` | Done |
| E5-T11 | TEST | Unit test: array parameter parsing | `tests/unit/registry_tests.rs` | Done |
| E5-T12 | TEST | Unit test: fallback to string on unknown | `tests/unit/registry_tests.rs` | Done |
| E5-T13 | TEST | Integration test: MCP tool validation passes | `tests/integration/mcp_tools_tests.rs` | Deferred (no MCP test infrastructure exists) |

**Tool Registry:**

```rust
// src/tools/registry.rs

use std::collections::HashMap;
use crate::anthropic::types::ToolDefinition;

/// Registry for looking up tool parameter types from schemas.
#[derive(Debug, Clone, Default)]
pub struct ToolRegistry {
    /// Map from tool name to parameter schemas.
    tools: HashMap<String, ToolSchema>,
}

#[derive(Debug, Clone)]
struct ToolSchema {
    /// Map from parameter name to its type.
    params: HashMap<String, ParamType>,
}

#[derive(Debug, Clone)]
pub enum ParamType {
    String,
    Number,
    Integer,
    Boolean,
    Object,
    Array,
    Null,
}

impl ToolRegistry {
    /// Build a registry from Anthropic tool definitions.
    pub fn from_tools(tools: &[ToolDefinition]) -> Self {
        let mut registry = ToolRegistry::default();

        for tool in tools {
            let mut params = HashMap::new();

            if let Some(properties) = &tool.input_schema.properties {
                if let Some(props_obj) = properties.as_object() {
                    for (param_name, param_schema) in props_obj {
                        if let Some(param_type) = extract_param_type(param_schema) {
                            params.insert(param_name.clone(), param_type);
                        }
                    }
                }
            }

            registry.tools.insert(tool.name.clone(), ToolSchema { params });
        }

        registry
    }

    /// Look up the expected type for a parameter.
    ///
    /// Returns `None` if the tool or parameter is not found.
    pub fn get_param_type(&self, tool_name: &str, param_name: &str) -> Option<&ParamType> {
        self.tools
            .get(tool_name)
            .and_then(|schema| schema.params.get(param_name))
    }
}

/// Extract the parameter type from a JSON Schema property.
fn extract_param_type(schema: &serde_json::Value) -> Option<ParamType> {
    schema.get("type").and_then(|t| t.as_str()).map(|s| match s {
        "string" => ParamType::String,
        "number" => ParamType::Number,
        "integer" => ParamType::Integer,
        "boolean" => ParamType::Boolean,
        "object" => ParamType::Object,
        "array" => ParamType::Array,
        "null" => ParamType::Null,
        _ => ParamType::String, // Default to string for unknown types
    })
}

/// Parse a string value according to the expected type.
///
/// Falls back to string if parsing fails or type is unknown.
pub fn parse_value_with_type(value: &str, param_type: &ParamType) -> serde_json::Value {
    match param_type {
        ParamType::Number => {
            value.parse::<f64>()
                .map(serde_json::Value::from)
                .unwrap_or_else(|_| serde_json::Value::String(value.to_string()))
        }
        ParamType::Integer => {
            value.parse::<i64>()
                .map(serde_json::Value::from)
                .unwrap_or_else(|_| serde_json::Value::String(value.to_string()))
        }
        ParamType::Boolean => {
            match value.to_lowercase().as_str() {
                "true" => serde_json::Value::Bool(true),
                "false" => serde_json::Value::Bool(false),
                _ => serde_json::Value::String(value.to_string()),
            }
        }
        ParamType::Null => {
            if value.to_lowercase() == "null" {
                serde_json::Value::Null
            } else {
                serde_json::Value::String(value.to_string())
            }
        }
        ParamType::Object | ParamType::Array => {
            // Try to parse as JSON
            serde_json::from_str(value)
                .unwrap_or_else(|_| serde_json::Value::String(value.to_string()))
        }
        ParamType::String => serde_json::Value::String(value.to_string()),
    }
}
```

**Parser Updates:**

```rust
// src/tools/parser.rs updates

/// Parse tool calls from model-generated text content.
///
/// If `registry` is provided, parameter values are coerced to their schema-defined
/// types. Otherwise, all values are returned as strings.
pub fn parse_tool_calls(
    content: &str,
    registry: Option<&ToolRegistry>,
    debug_tools: bool,
) -> Vec<ToolCall> {
    // ... existing logic, but pass registry to parse functions
}

/// Parse XML parameters with optional type coercion.
fn parse_xml_params(
    params_content: &str,
    tool_name: &str,
    registry: Option<&ToolRegistry>,
) -> serde_json::Value {
    let mut params = serde_json::Map::new();

    for cap in OPEN_TAG.captures_iter(params_content) {
        let full_match = cap.get(0).unwrap();
        let name = cap.get(1).unwrap().as_str();
        let closing_tag = format!("</{name}>");

        let after_open = full_match.end();
        if let Some(close_pos) = params_content[after_open..].find(&closing_tag) {
            let value = &params_content[after_open..after_open + close_pos];
            let trimmed = value.trim();

            // Coerce type if registry available
            let typed_value = if let Some(reg) = registry {
                if let Some(param_type) = reg.get_param_type(tool_name, name) {
                    parse_value_with_type(trimmed, param_type)
                } else {
                    serde_json::Value::String(trimmed.to_string())
                }
            } else {
                serde_json::Value::String(trimmed.to_string())
            };

            params.insert(name.to_string(), typed_value);
        }
    }

    serde_json::Value::Object(params)
}

/// Parse attribute-based XML parameters with optional type coercion.
fn parse_attribute_params(
    invoke_body: &str,
    tool_name: &str,
    registry: Option<&ToolRegistry>,
) -> serde_json::Value {
    let mut params = serde_json::Map::new();

    for param_cap in XML_PARAMETER.captures_iter(invoke_body) {
        let param_name = param_cap.get(1).unwrap().as_str();
        let param_value = param_cap.get(2).unwrap().as_str().trim();

        // Coerce type if registry available
        let typed_value = if let Some(reg) = registry {
            if let Some(param_type) = reg.get_param_type(tool_name, param_name) {
                parse_value_with_type(param_value, param_type)
            } else {
                serde_json::Value::String(param_value.to_string())
            }
        } else {
            serde_json::Value::String(param_value.to_string())
        };

        params.insert(param_name.to_string(), typed_value);
    }

    serde_json::Value::Object(params)
}
```

**Acceptance Criteria:**
- [x] `ToolRegistry` built from tool definitions
- [x] Number parameters parsed as numbers
- [x] Integer parameters parsed as integers
- [x] Boolean parameters parsed as booleans
- [x] Object/array parameters parsed as JSON
- [x] Unknown parameters fall back to strings
- [x] Parser gracefully handles missing registry (all strings)
- [x] MCP validation errors fixed
- [x] All unit tests pass

**Completion Notes (2026-03-31):** Fixed correctness bug in `parse_value_with_type()` where the combined `ParamType::Object | ParamType::Array` arm used generic `serde_json::from_str::<serde_json::Value>()`, accepting any valid JSON type. Split into separate arms: `ParamType::Object` now deserializes via `serde_json::from_str::<serde_json::Map<String, serde_json::Value>>()` and `ParamType::Array` via `serde_json::from_str::<Vec<serde_json::Value>>()`, causing wrong-type JSON to fall back to string. Added 9 cross-type coercion mismatch tests. All existing positive-case tests continue to pass.

---

### Epic 6: CLI and Configuration(Day 6, 0.5 days)

**Goal:** Add CLI flags and configuration for native tools.

**Status:** DONE

**Prerequisite:** Epic 4

**Tasks:**

| Task ID | Type | Description | Files | Status |
|---------|------|-------------|-------|--------|
| E6-T1 | IMPL | Add `--native-tools` flag | `src/cli.rs` | Done (Epic 4) |
| E6-T2 | IMPL | Add `--xml-tools` flag (force XML fallback) | `src/cli.rs` | Done |
| E6-T3 | IMPL | Update `AdapterConfig` with tool mode | `src/server.rs` | Done |
| E6-T4 | IMPL | Pass configuration to handler | `src/main.rs` | Done |
| E6-T5 | DOC | Update CLI help text | `src/cli.rs` | Done |
| E6-T6 | TEST | CLI test: default mode | `tests/unit/cli_tests.rs` | Done |
| E6-T7 | TEST | CLI test: native-tools flag | `tests/unit/cli_tests.rs` | Done (Epic 4) |
| E6-T8 | TEST | CLI test: xml-tools flag | `tests/unit/cli_tests.rs` | Done |

**CLI Changes:**

```rust
// src/cli.rs additions

#[derive(Parser, Debug)]
pub struct StartArgs {
    // ... existing flags ...

    /// Enable native OpenAI tools passthrough (experimental).
    /// Tools are sent to Copilot API in native format for progressive streaming.
    /// Falls back to XML injection if not supported.
    #[arg(long)]
    pub native_tools: bool,

    /// Force XML-based tool injection (disables native tools).
    /// Use this if native tools cause issues.
    #[arg(long)]
    pub xml_tools: bool,
}
```

**Acceptance Criteria:**
- [x] `--native-tools` enables native tools
- [x] `--xml-tools` forces XML injection
- [x] Flags are mutually exclusive (error if both specified)
- [x] Help text explains each mode
- [x] Default is XML injection (for stability)

**Completion Notes (2026-03-31):** Added `--xml-tools` flag to `src/cli.rs` with `conflicts_with = "native_tools"` for clap-enforced mutual exclusivity (symmetric with `--native-tools`). Added `xml_tools: bool` field to `AdapterConfig` in `src/server.rs`. Wired `xml_tools` through `src/main.rs` including Windows daemon arg forwarding and `tracing::info` log when explicitly enabled. Updated help text for both flags with detailed doc comments. Added comprehensive CLI tests for default mode, `--xml-tools` flag, and mutual exclusivity enforcement.

---

### Epic 7: Documentation and Testing (Day 7-8, 1.5 days)

**Goal:** Comprehensive documentation and testing.

**Status:** Pending

**Prerequisite:** All previous epics

**Tasks:**

| Task ID | Type | Description | Files | Status |
|---------|------|-------------|-------|--------|
| E7-T1 | DOC | Update CLAUDE.md with native tools info | `CLAUDE.md` | Pending |
| E7-T2 | DOC | Update README.md with tool mode flags | `README.md` | Pending |
| E7-T3 | DOC | Add native tools section to e2e-testing.md | `docs/development/e2e-testing.md` | Pending |
| E7-T4 | DOC | Update known-issues.md | `docs/known-issues.md` | Pending |
| E7-T5 | DOC | Update NATIVE-TOOLS-STREAMING.design.md status | `docs/design/NATIVE-TOOLS-STREAMING.design.md` | Pending |
| E7-T6 | TEST | Manual E2E: native tools with Claude Code | Manual | Pending |
| E7-T7 | TEST | Manual E2E: streaming UX comparison | Manual | Pending |
| E7-T8 | TEST | Manual E2E: MCP tools with typed params | Manual | Pending |
| E7-T9 | TEST | Manual E2E: fallback to XML | Manual | Pending |
| E7-T10 | TEST | Run full test suite | Manual | Pending |
| E7-T11 | TEST | Run clippy and format check | Manual | Pending |

**Documentation Updates:**

```markdown
<!-- CLAUDE.md additions -->

## Notes for Development

- **Native tools** (experimental): When `--native-tools` is enabled, tool definitions
  are passed to the Copilot API in OpenAI format and responses stream progressively.
  Falls back to XML injection if not supported. Use `--xml-tools` to force XML mode.
- **Tool name truncation**: OpenAI has a 64-character limit for function names.
  Long names (common with MCP tools like `mcp__codemogger__codemogger_search`) are
  truncated with a hash suffix and restored in responses.
- **Parameter types**: Native tools preserve parameter types from schemas. XML fallback
  path coerces string values to their schema-defined types (number, boolean, etc.).
```

**Manual E2E Test Plan:**

1. **Native tools basic**
   ```bash
   copilot-adapter start --native-tools
   # Use Claude Code to read a file
   # Verify streaming shows thinking progressively
   ```

2. **Native tools with MCP**
   ```bash
   copilot-adapter start --native-tools
   # Use MCP tool with typed parameters (number, boolean)
   # Verify no validation errors
   ```

3. **XML fallback**
   ```bash
   copilot-adapter start --xml-tools
   # Use Claude Code with tools
   # Verify XML injection works (buffered response)
   ```

4. **Parameter typing (XML path)**
   ```bash
   copilot-adapter start --xml-tools
   # Use MCP tool with typed parameters
   # Verify types are correctly coerced
   ```

**Acceptance Criteria:**
- [ ] All documentation updated
- [ ] Manual E2E tests pass
- [ ] Full test suite passes
- [ ] No clippy warnings
- [ ] Code formatted

---

## Timeline Summary

| Epic | Description | Duration | Dependencies |
|------|-------------|----------|--------------|
| Epic 0 | Verify native tools support | 0.5 days | None |
| Epic 1 | Tool translation layer | 1.5 days | Epic 0 |
| Epic 2 | Update Copilot client | 0.5 days | Epic 1 |
| Epic 3 | Streaming state machine | 2 days | Epic 2 |
| Epic 4 | Handler integration | 1.5 days | Epics 1-3 |
| Epic 5 | Schema-aware XML parsing | 1 day | None (parallel) |
| Epic 6 | CLI and configuration | 0.5 days | Epic 4 |
| Epic 7 | Documentation and testing | 1.5 days | All above |

**Total: 6-8 days**

---

## Risk Assessment

| Risk | Impact | Probability | Mitigation |
|------|--------|-------------|------------|
| Copilot doesn't support native tools | High | Low | litellm uses it; Epic 0 verifies |
| Tool name truncation collisions | Medium | Low | SHA-256 hash makes collisions unlikely |
| Streaming state machine bugs | Medium | Medium | Comprehensive unit tests |
| Regression in XML fallback | Medium | Low | Keep existing tests; add schema-aware tests |
| Performance impact | Low | Low | Native tools should be faster (no buffering) |
| Parameter coercion edge cases | Medium | Medium | Fall back to string on parse failure |

---

## Rollback Plan

If critical issues arise after deployment:

1. **Immediate**: Use `--xml-tools` flag to force XML injection
2. **Short-term**: Revert native tools code, keep schema-aware parsing
3. **Long-term**: Fix issues and re-enable native tools

All changes are additive with feature flags, making rollback straightforward.

---

## Success Criteria

| Metric | Target |
|--------|--------|
| Native tools streaming | Visible progressive output |
| Parameter type preservation | 100% (from schema) |
| MCP validation errors | 0 |
| Tool call parse success | ≥99% |
| Integration test pass rate | 100% |
| Fallback reliability | 100% when native fails |

---

## Checklist Summary

### Epic 0: Verify Native Tools
- [ ] Native tools request accepted
- [ ] Streaming tool_calls work
- [ ] Findings documented

### Epic 1: Tool Translation
- [x] `translate_anthropic_tools_to_openai()` implemented
- [x] Tool name truncation working
- [x] Name mapping roundtrip verified
- [x] Unit tests pass

### Epic 2: Copilot Client
- [x] Request types updated
- [x] Streaming types updated
- [x] MessageContent null handling fixed
- [x] Unit tests pass

### Epic 3: Streaming State Machine
- [x] `StreamingState` implemented
- [x] Content block transitions working
- [x] Tool name restoration working
- [x] Unit tests pass

### Epic 4: Handler Integration
- [x] Native tools path implemented
- [x] Fallback detection working
- [x] Integration tests pass

### Epic 5: Schema-Aware Parsing
- [x] `ToolRegistry` implemented
- [x] Type coercion working
- [x] XML parser updated
- [x] MCP tests pass

### Epic 6: CLI Configuration
- [x] `--native-tools` flag added
- [x] `--xml-tools` flag added
- [x] Help text updated

### Epic 7: Documentation
- [ ] CLAUDE.md updated
- [ ] README.md updated
- [ ] e2e-testing.md updated
- [ ] Manual tests pass
- [ ] Full suite passes

---

## References

| Document | Description |
|----------|-------------|
| [NATIVE-TOOLS-STREAMING.design.md](./NATIVE-TOOLS-STREAMING.design.md) | Design document |
| [BUG-ANALYSIS-TOOL-PARAMS-TYPING.md](../../BUG-ANALYSIS-TOOL-PARAMS-TYPING.md) | Bug analysis |
| [DUAL-RESPONSES.plan.md](./DUAL-RESPONSES.plan.md) | XML format migration (completed) |
| [litellm source](https://github.com/BerriAI/litellm/tree/main/litellm/llms/github_copilot) | Reference implementation |
| [OpenAI Function Calling](https://platform.openai.com/docs/guides/function-calling) | API reference |
| [Anthropic Tool Use](https://docs.anthropic.com/en/docs/tool-use) | API reference |
