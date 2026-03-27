# Tools/Functions Support for Copilot Adapter — Implementation Plan

**Status:** Ready for Implementation
**Date:** 2026-03-27
**Based on:** [TOOLS-SUPPORT.design.md](./TOOLS-SUPPORT.design.md)
**Prerequisite:** Core adapter implementation — **COMPLETE**

---

## Executive Summary

This plan implements experimental tool/function calling support for the GitHub Copilot Adapter via prompt injection. Since GitHub Copilot's upstream API does not natively support the OpenAI `tools`/`functions` parameters or Anthropic's `tools` parameter, this feature injects tool definitions into the system prompt and parses tool calls from the model's text response.

The implementation adds:
- `--experimental-tools` flag to enable the feature (opt-in)
- Tool definition injection into system prompts (JSON format)
- Tool call parsing from model responses
- Support for both OpenAI (`/v1/chat/completions`) and Anthropic (`/v1/messages`) API formats
- Streaming support with tool call buffering

This enables Claude Code users to use native tool calling (file operations, bash commands, etc.) through the adapter.

---

## Background

### Current State

The adapter is fully functional with:
- OpenAI-compatible `/v1/chat/completions` endpoint
- Anthropic-compatible `/v1/messages` endpoint with bidirectional translation
- SSE streaming support for both endpoints
- Token management with auto-refresh

However, **tool/function parameters are silently ignored**, causing Claude Code to generate text responses instead of executing tools.

### Target State

When `--experimental-tools` is enabled:
- Tool definitions are accepted and injected into the system prompt
- Model responses are parsed for tool calls in JSON format
- Tool calls are returned in the appropriate format (OpenAI `tool_calls` or Anthropic `tool_use` blocks)
- Claude Code can execute local tools (bash, file read/write, etc.)

---

## Problem Statement

Claude Code sends requests with a `tools` array containing tool definitions. The adapter currently:
1. Silently ignores the `tools` field (serde skips unknown fields)
2. Forwards the request without tool information
3. Returns a text-only response without `tool_calls`
4. Claude Code displays the text instead of executing tools

Example failing interaction:
```
User: What is my current working directory?
Claude: Your current working directory is shown above...  # No actual execution
```

---

## Goals and Non-Goals

### Goals

| ID | Goal | Success Criteria |
|----|------|------------------|
| G1 | Accept `tools` parameter in requests | Requests with `tools` don't fail; definitions captured |
| G2 | Inject tool definitions into prompts | System prompt contains formatted tool descriptions |
| G3 | Parse tool calls from responses | JSON tool calls extracted from model text output |
| G4 | Return proper `tool_calls` format | OpenAI format for `/v1/chat/completions`, Anthropic format for `/v1/messages` |
| G5 | Support tool results in conversations | `tool` role messages (OpenAI) and `tool_result` blocks (Anthropic) handled |
| G6 | Support streaming with tools | Tool calls detected and returned during streaming |
| G7 | Feature is opt-in | Disabled by default; enabled with `--experimental-tools` flag |

### Non-Goals

| ID | Non-Goal | Rationale |
|----|----------|-----------|
| NG1 | 100% parsing reliability | Prompt injection is inherently imperfect; graceful degradation preferred |
| NG2 | `tool_choice` parameter support | Start with "auto" behavior only |
| NG3 | `parallel_tool_calls` support | Sequential tool calls sufficient for initial version |
| NG4 | Native Copilot tool support | Copilot doesn't support it; this is a workaround |

---

## Requirements

### Functional Requirements

| ID | Requirement | Source |
|----|-------------|--------|
| FR1 | `--experimental-tools` flag enables tool support | Design: Recommended Approach |
| FR2 | `ChatCompletionRequest` accepts optional `tools` array | OpenAI API spec |
| FR3 | `AnthropicRequest` accepts optional `tools` array | Anthropic API spec |
| FR4 | Tool definitions injected into system prompt as JSON | Design: Option C |
| FR5 | Model instructed to output tool calls in JSON format | Design: Tool Usage Instructions |
| FR6 | Tool calls parsed from response using regex/JSON extraction | Design: Response Parsing |
| FR7 | Parsed tool calls returned in `choices[0].message.tool_calls` (OpenAI) | OpenAI API spec |
| FR8 | Parsed tool calls returned as `tool_use` content blocks (Anthropic) | Anthropic API spec |
| FR9 | `tool` role messages translated appropriately | OpenAI API spec |
| FR10 | `tool_result` content blocks translated appropriately | Anthropic API spec |
| FR11 | Tool calls stripped from visible `content` field | Design: Open Question #4 |
| FR12 | Without `--experimental-tools`, requests with tools return 400 error | Current behavior preserved |

### Non-Functional Requirements

| ID | Requirement | Target |
|----|-------------|--------|
| NFR1 | Parsing latency overhead | < 10ms for typical responses |
| NFR2 | Memory overhead for tool injection | < 1MB per request |
| NFR3 | Streaming tool detection latency | Tool calls detected within 100ms of completion |

---

## Proposed Architecture

### Component Overview

```
┌──────────────────────────────────────────────────────────────────────────┐
│                         copilot-adapter                                   │
│                                                                          │
│  ┌────────────────┐     ┌─────────────────────────────────────────────┐  │
│  │ CLI (clap)     │     │ AppState                                    │  │
│  │ + experimental │     │ + config: AdapterConfig                     │  │
│  │   _tools flag  │     │   + experimental_tools: bool                │  │
│  └────────────────┘     └─────────────────────────────────────────────┘  │
│                                                                          │
│  ┌────────────────────────────────────────────────────────────────────┐  │
│  │ Handlers                                                           │  │
│  │                                                                    │  │
│  │  /v1/chat/completions ───┬──► Tool Injector ───► Copilot Client   │  │
│  │                          │                              │          │  │
│  │  /v1/messages ───────────┘                              ▼          │  │
│  │                                                   Response Parser  │  │
│  │                                                         │          │  │
│  │                          ◄──────────────────────────────┘          │  │
│  └────────────────────────────────────────────────────────────────────┘  │
│                                                                          │
│  ┌────────────────────────────────────────────────────────────────────┐  │
│  │ src/tools/                                                         │  │
│  │                                                                    │  │
│  │  ┌──────────────┐  ┌──────────────┐  ┌──────────────────────────┐ │  │
│  │  │ types.rs     │  │ injector.rs  │  │ parser.rs                │ │  │
│  │  │ • Tool       │  │ • inject()   │  │ • parse_tool_calls()     │ │  │
│  │  │ • ToolCall   │  │ • format()   │  │ • strip_tool_calls()     │ │  │
│  │  │ • ToolResult │  │              │  │ • ToolCallMatch          │ │  │
│  │  └──────────────┘  └──────────────┘  └──────────────────────────┘ │  │
│  └────────────────────────────────────────────────────────────────────┘  │
└──────────────────────────────────────────────────────────────────────────┘
```

### Data Flow

```
Request with tools
        │
        ▼
┌───────────────────────────────┐
│ Handler receives request      │
│ • Check --experimental-tools  │
│ • If disabled → 400 error     │
└───────────────────────────────┘
        │ enabled
        ▼
┌───────────────────────────────┐
│ Tool Injector                 │
│ • Extract tools from request  │
│ • Format as JSON prompt       │
│ • Prepend to system message   │
│ • Remove tools from request   │
└───────────────────────────────┘
        │
        ▼
┌───────────────────────────────┐
│ Copilot Client                │
│ • Send modified request       │
│ • Receive text response       │
└───────────────────────────────┘
        │
        ▼
┌───────────────────────────────┐
│ Response Parser               │
│ • Find JSON tool calls        │
│ • Extract name + arguments    │
│ • Generate tool call IDs      │
│ • Strip from content          │
└───────────────────────────────┘
        │
        ▼
┌───────────────────────────────┐
│ Response Builder              │
│ • Add tool_calls to response  │
│ • Format for OpenAI/Anthropic │
└───────────────────────────────┘
        │
        ▼
    Response
```

### Key Design Decisions

| Decision | Rationale |
|----------|-----------|
| **JSON format for tool prompts** | GPT-4 excels at JSON generation; easier to parse than XML |
| **Opt-in via flag** | Experimental feature; users should understand limitations |
| **Regex + JSON parsing** | Robust to partial matches; handles fenced code blocks |
| **Strip tool calls from content** | Clean separation between text and structured tool calls |
| **Same injection for both APIs** | Tool injection happens before format translation |

---

## Dependencies

### New Dependencies

| Crate | Version | Purpose |
|-------|---------|---------|
| `regex` | 1.x | Tool call pattern matching |
| `lazy_static` or `once_cell` | — | Compile regex once (may already be transitive) |

### Sequencing Constraints

1. Epic 1 (Types) must complete before Epics 2, 3
2. Epic 2 (Injection) can proceed in parallel with Epic 3 (Parsing) after Epic 1
3. Epic 4 (OpenAI Integration) depends on Epics 2, 3
4. Epic 5 (Anthropic Integration) depends on Epics 2, 3, and partially on Epic 4
5. Epic 6 (Streaming) depends on Epics 4, 5
6. Epic 7 (Testing) depends on all previous epics

---

## Impact Analysis

### Files Modified

| File Path | Changes |
|-----------|---------|
| `Cargo.toml` | Add `regex` dependency |
| `src/cli.rs` | Add `--experimental-tools` flag to `Start` command |
| `src/server.rs` | Add `AdapterConfig` to `AppState`; pass to handlers |
| `src/copilot/types.rs` | Add `tools`, `tool_calls`, `tool_choice` fields to request/response types |
| `src/anthropic/types.rs` | Add `tools`, `tool_use`, `tool_result` content block types |
| `src/handlers/chat.rs` | Integrate tool injection/parsing for OpenAI endpoint |
| `src/handlers/messages.rs` | Integrate tool injection/parsing for Anthropic endpoint |
| `src/lib.rs` | Export new `tools` module |
| `src/main.rs` | Pass config to server; handle flag |

### Files Created

| File Path | Purpose |
|-----------|---------|
| `src/tools/mod.rs` | Module exports |
| `src/tools/types.rs` | Tool, ToolCall, Function, ToolResult types |
| `src/tools/injector.rs` | Tool prompt formatting and injection |
| `src/tools/parser.rs` | Tool call extraction from responses |
| `tests/unit/tools_types_tests.rs` | Type serialization tests |
| `tests/unit/tools_injector_tests.rs` | Injection logic tests |
| `tests/unit/tools_parser_tests.rs` | Parsing logic tests |
| `tests/integration/tools_chat_tests.rs` | OpenAI endpoint integration tests |
| `tests/integration/tools_messages_tests.rs` | Anthropic endpoint integration tests |

---

## Risks and Mitigations

| # | Risk | Likelihood | Impact | Mitigation |
|---|------|------------|--------|------------|
| R1 | Model doesn't follow tool call format | High | Medium | Graceful degradation; return response as-is if no tools parsed |
| R2 | Tool calls split across streaming chunks | Medium | Medium | Buffer content until complete JSON found |
| R3 | False positive tool call detection | Low | Low | Require specific JSON structure; use fenced code blocks |
| R4 | Performance impact from regex parsing | Low | Low | Compile regex once; parsing is O(n) on response length |
| R5 | Complex nested tool arguments fail to parse | Medium | Medium | Use serde_json for argument parsing; accept any valid JSON |

---

## Implementation Plan

### Epic 1: Tool Types and Data Structures

**Status:** COMPLETE

**Goal:** Define all types needed for tool support in both OpenAI and Anthropic formats.

**Prerequisites:** None

**Tasks:**

| Task ID | Type | Description | Files | Status |
|---------|------|-------------|-------|--------|
| E1-T1 | IMPL | Create `src/tools/mod.rs` with module exports | `src/tools/mod.rs` | DONE |
| E1-T2 | IMPL | Create `src/tools/types.rs` with `Tool`, `Function`, `FunctionParameters` structs matching OpenAI schema | `src/tools/types.rs` | DONE |
| E1-T3 | IMPL | Add `ToolCall`, `FunctionCall` structs for response tool calls | `src/tools/types.rs` | DONE |
| E1-T4 | IMPL | Add `tools` and `tool_choice` fields to `ChatCompletionRequest` | `src/copilot/types.rs` | DONE |
| E1-T5 | IMPL | Add `tool_calls` field to `Message` struct | `src/copilot/types.rs` | DONE |
| E1-T6 | IMPL | Add `tool_calls` field to `Choice` and `ChunkChoice` for streaming | `src/copilot/types.rs` | DONE |
| E1-T7 | IMPL | Add Anthropic `ToolDefinition`, `InputSchema` types | `src/anthropic/types.rs` | DONE |
| E1-T8 | IMPL | Add `ToolUseBlock`, `ToolResultBlock` content block variants | `src/anthropic/types.rs` | DONE |
| E1-T9 | IMPL | Add `tools` field to `AnthropicRequest` | `src/anthropic/types.rs` | DONE |
| E1-T10 | IMPL | Export `tools` module from `src/lib.rs` | `src/lib.rs` | DONE |
| E1-T11 | TEST | Unit tests for Tool type serialization/deserialization | `tests/unit/tools_types_tests.rs` | DONE |
| E1-T12 | TEST | Unit tests for ToolCall type serialization | `tests/unit/tools_types_tests.rs` | DONE |
| E1-T13 | TEST | Unit tests for Anthropic tool types | `tests/unit/tools_types_tests.rs` | DONE |

**Acceptance Criteria:**
- [x] `Tool` struct deserializes from OpenAI tool definition JSON
- [x] `ToolCall` struct serializes to OpenAI tool_calls format
- [x] Anthropic `ToolUseBlock` serializes correctly
- [x] All types have appropriate `skip_serializing_if` for optional fields

---

### Epic 2: Tool Injection

**Goal:** Implement system prompt injection with tool definitions.

**Prerequisites:** Epic 1

**Tasks:**

| Task ID | Type | Description | Files | Status |
|---------|------|-------------|-------|--------|
| E2-T1 | IMPL | Create `src/tools/injector.rs` with module structure | `src/tools/injector.rs` | |
| E2-T2 | IMPL | Implement `format_tools_as_json()` to convert `Vec<Tool>` to JSON prompt | `src/tools/injector.rs` | |
| E2-T3 | IMPL | Define `TOOL_USAGE_INSTRUCTIONS` constant with call format instructions | `src/tools/injector.rs` | |
| E2-T4 | IMPL | Implement `inject_tools_into_messages()` to prepend/append to system message | `src/tools/injector.rs` | |
| E2-T5 | IMPL | Handle case where no system message exists (create one) | `src/tools/injector.rs` | |
| E2-T6 | IMPL | Implement `translate_tool_messages()` to handle `tool` role messages | `src/tools/injector.rs` | |
| E2-T7 | TEST | Unit test: tools formatted as valid JSON | `tests/unit/tools_injector_tests.rs` | |
| E2-T8 | TEST | Unit test: injection prepends to existing system message | `tests/unit/tools_injector_tests.rs` | |
| E2-T9 | TEST | Unit test: injection creates system message if missing | `tests/unit/tools_injector_tests.rs` | |
| E2-T10 | TEST | Unit test: tool role messages translated to user messages with results | `tests/unit/tools_injector_tests.rs` | |

**Acceptance Criteria:**
- [ ] Tool definitions formatted as readable JSON in prompt
- [ ] Instructions tell model how to format tool calls
- [ ] Existing system message content preserved
- [ ] Tool result messages converted to appropriate format

---

### Epic 3: Tool Call Parsing

**Goal:** Extract tool calls from model text responses.

**Prerequisites:** Epic 1

**Tasks:**

| Task ID | Type | Description | Files | Status |
|---------|------|-------------|-------|--------|
| E3-T1 | IMPL | Add `regex` dependency to `Cargo.toml` | `Cargo.toml` | |
| E3-T2 | IMPL | Create `src/tools/parser.rs` with module structure | `src/tools/parser.rs` | |
| E3-T3 | IMPL | Define regex pattern for JSON tool calls in fenced code blocks | `src/tools/parser.rs` | |
| E3-T4 | IMPL | Define regex pattern for inline JSON tool calls | `src/tools/parser.rs` | |
| E3-T5 | IMPL | Implement `parse_tool_calls()` returning `Vec<ToolCall>` | `src/tools/parser.rs` | |
| E3-T6 | IMPL | Generate unique `call_xxx` IDs for each parsed tool call | `src/tools/parser.rs` | |
| E3-T7 | IMPL | Implement `strip_tool_calls()` to remove tool call text from content | `src/tools/parser.rs` | |
| E3-T8 | IMPL | Handle multiple tool calls in single response | `src/tools/parser.rs` | |
| E3-T9 | TEST | Unit test: parse single tool call from fenced code block | `tests/unit/tools_parser_tests.rs` | |
| E3-T10 | TEST | Unit test: parse multiple tool calls | `tests/unit/tools_parser_tests.rs` | |
| E3-T11 | TEST | Unit test: parse tool call with complex nested arguments | `tests/unit/tools_parser_tests.rs` | |
| E3-T12 | TEST | Unit test: no tool calls found returns empty vec | `tests/unit/tools_parser_tests.rs` | |
| E3-T13 | TEST | Unit test: malformed JSON gracefully skipped | `tests/unit/tools_parser_tests.rs` | |
| E3-T14 | TEST | Unit test: strip_tool_calls removes tool call text | `tests/unit/tools_parser_tests.rs` | |

**Acceptance Criteria:**
- [ ] Tool calls in ```json blocks parsed correctly
- [ ] Tool calls with/without fencing both detected
- [ ] Arguments preserved as raw JSON string
- [ ] Multiple tool calls extracted in order
- [ ] Invalid JSON skipped without error

---

### Epic 4: OpenAI Endpoint Integration

**Goal:** Integrate tool injection and parsing into `/v1/chat/completions` handler.

**Prerequisites:** Epics 1, 2, 3

**Tasks:**

| Task ID | Type | Description | Files | Status |
|---------|------|-------------|-------|--------|
| E4-T1 | IMPL | Add `--experimental-tools` flag to CLI `Start` command | `src/cli.rs` | |
| E4-T2 | IMPL | Create `AdapterConfig` struct with `experimental_tools: bool` | `src/server.rs` | |
| E4-T3 | IMPL | Add `config: AdapterConfig` to `AppState` | `src/server.rs` | |
| E4-T4 | IMPL | Pass config from CLI to server startup | `src/main.rs` | |
| E4-T5 | IMPL | In `chat_completions` handler: check for tools in request | `src/handlers/chat.rs` | |
| E4-T6 | IMPL | If tools present and flag disabled: return 400 error | `src/handlers/chat.rs` | |
| E4-T7 | IMPL | If tools present and flag enabled: call `inject_tools_into_messages()` | `src/handlers/chat.rs` | |
| E4-T8 | IMPL | After response: call `parse_tool_calls()` on content | `src/handlers/chat.rs` | |
| E4-T9 | IMPL | If tool calls found: add to `message.tool_calls`, strip from content | `src/handlers/chat.rs` | |
| E4-T10 | IMPL | Handle `tool` role messages in request (translate to user message with result) | `src/handlers/chat.rs` | |
| E4-T11 | TEST | Integration test: request with tools and flag disabled returns 400 | `tests/integration/tools_chat_tests.rs` | |
| E4-T12 | TEST | Integration test: request with tools and flag enabled succeeds | `tests/integration/tools_chat_tests.rs` | |
| E4-T13 | TEST | Integration test: tool call parsed from mock response | `tests/integration/tools_chat_tests.rs` | |
| E4-T14 | TEST | Integration test: tool role message handled correctly | `tests/integration/tools_chat_tests.rs` | |

**Acceptance Criteria:**
- [ ] `--experimental-tools` flag recognized by CLI
- [ ] Requests with tools fail with 400 when flag disabled
- [ ] Requests with tools succeed when flag enabled
- [ ] Tool calls appear in response `choices[0].message.tool_calls`
- [ ] Tool call text stripped from `choices[0].message.content`

---

### Epic 5: Anthropic Endpoint Integration

**Goal:** Integrate tool support into `/v1/messages` handler with Anthropic format translation.

**Prerequisites:** Epics 1, 2, 3, Epic 4 (for config)

**Tasks:**

| Task ID | Type | Description | Files | Status |
|---------|------|-------------|-------|--------|
| E5-T1 | IMPL | In `messages` handler: check for tools in request | `src/handlers/messages.rs` | |
| E5-T2 | IMPL | If tools present and flag disabled: return 400 error | `src/handlers/messages.rs` | |
| E5-T3 | IMPL | Translate Anthropic `tools` to internal Tool format | `src/anthropic/types.rs` | |
| E5-T4 | IMPL | Inject tools into translated OpenAI request | `src/handlers/messages.rs` | |
| E5-T5 | IMPL | Parse tool calls from response | `src/handlers/messages.rs` | |
| E5-T6 | IMPL | Convert `ToolCall` to Anthropic `ToolUseBlock` content | `src/anthropic/types.rs` | |
| E5-T7 | IMPL | Return tool_use blocks in Anthropic response content array | `src/handlers/messages.rs` | |
| E5-T8 | IMPL | Handle `tool_result` content blocks in request (translate to tool role message) | `src/anthropic/types.rs` | |
| E5-T9 | TEST | Integration test: Anthropic request with tools and flag disabled returns 400 | `tests/integration/tools_messages_tests.rs` | |
| E5-T10 | TEST | Integration test: Anthropic request with tools succeeds | `tests/integration/tools_messages_tests.rs` | |
| E5-T11 | TEST | Integration test: tool_use block in response | `tests/integration/tools_messages_tests.rs` | |
| E5-T12 | TEST | Integration test: tool_result in request handled | `tests/integration/tools_messages_tests.rs` | |

**Acceptance Criteria:**
- [ ] Anthropic `tools` array accepted and processed
- [ ] Tool calls returned as `tool_use` content blocks
- [ ] `tool_result` blocks translated and forwarded correctly
- [ ] Response `stop_reason` is `"tool_use"` when tool called

---

### Epic 6: Streaming Support

**Goal:** Support tool call detection and return in streaming responses.

**Prerequisites:** Epics 4, 5

**Tasks:**

| Task ID | Type | Description | Files | Status |
|---------|------|-------------|-------|--------|
| E6-T1 | IMPL | Buffer streaming content for tool call detection | `src/handlers/chat.rs` | |
| E6-T2 | IMPL | Detect complete tool call JSON in buffered content | `src/handlers/chat.rs` | |
| E6-T3 | IMPL | Emit tool_calls in final chunk when detected | `src/handlers/chat.rs` | |
| E6-T4 | IMPL | Add `tool_calls` field to `ChunkDelta` for streaming | `src/copilot/types.rs` | |
| E6-T5 | IMPL | Buffer streaming content for Anthropic tool detection | `src/handlers/messages.rs` | |
| E6-T6 | IMPL | Emit `tool_use` content block in Anthropic streaming | `src/handlers/messages.rs` | |
| E6-T7 | IMPL | Add `ToolUseBlock` streaming event type | `src/anthropic/types.rs` | |
| E6-T8 | TEST | Integration test: OpenAI streaming with tool call | `tests/integration/tools_chat_tests.rs` | |
| E6-T9 | TEST | Integration test: Anthropic streaming with tool call | `tests/integration/tools_messages_tests.rs` | |
| E6-T10 | TEST | Integration test: streaming without tool call unaffected | `tests/integration/tools_chat_tests.rs` | |

**Acceptance Criteria:**
- [ ] Streaming responses with tool calls return proper `tool_calls` field
- [ ] Content is buffered until tool call JSON is complete
- [ ] Non-tool streaming responses unaffected by buffering
- [ ] Anthropic streaming emits correct event sequence for tool use

---

### Epic 7: Testing and Documentation

**Goal:** Comprehensive test coverage and documentation updates.

**Prerequisites:** Epics 1-6

**Tasks:**

| Task ID | Type | Description | Files | Status |
|---------|------|-------------|-------|--------|
| E7-T1 | TEST | Create mock Copilot responses with tool calls for tests | `tests/common/mock_copilot.rs` | |
| E7-T2 | TEST | End-to-end test: simple tool call (get_weather style) | `tests/integration/tools_e2e_tests.rs` | |
| E7-T3 | TEST | End-to-end test: multi-turn conversation with tool results | `tests/integration/tools_e2e_tests.rs` | |
| E7-T4 | TEST | End-to-end test: tool call with complex arguments | `tests/integration/tools_e2e_tests.rs` | |
| E7-T5 | TEST | Edge case test: response with no tool calls (graceful passthrough) | `tests/integration/tools_e2e_tests.rs` | |
| E7-T6 | TEST | Edge case test: malformed tool call JSON in response | `tests/integration/tools_e2e_tests.rs` | |
| E7-T7 | DOC | Update README.md with `--experimental-tools` documentation | `README.md` | |
| E7-T8 | DOC | Update CLAUDE.md with tools feature notes | `CLAUDE.md` | |
| E7-T9 | DOC | Add tools section to docs/e2e-testing.md | `docs/e2e-testing.md` | |
| E7-T10 | DOC | Update TOOLS-SUPPORT.design.md status to "Implemented" | `TOOLS-SUPPORT.design.md` | |

**Acceptance Criteria:**
- [ ] All unit tests pass
- [ ] All integration tests pass with mock servers
- [ ] README documents the experimental feature and limitations
- [ ] E2E testing procedures include tool scenarios

---

## Verification Plan

After implementation, verify tool support works correctly:

1. **Flag Test**
   ```bash
   copilot-adapter start --experimental-tools
   # Should start with tools enabled
   ```

2. **Rejection Test (flag disabled)**
   ```bash
   copilot-adapter start  # No flag
   curl -X POST http://127.0.0.1:6767/v1/chat/completions \
     -H "Content-Type: application/json" \
     -d '{"model":"gpt-4","messages":[{"role":"user","content":"hi"}],"tools":[{"type":"function","function":{"name":"test","parameters":{}}}]}'
   # Should return 400 error
   ```

3. **Tool Injection Test**
   ```bash
   copilot-adapter start --experimental-tools --log-level debug
   # Send request with tools, check logs for injected prompt
   ```

4. **Tool Call Parsing Test**
   ```bash
   curl -X POST http://127.0.0.1:6767/v1/chat/completions \
     -H "Content-Type: application/json" \
     -d '{"model":"gpt-4","messages":[{"role":"user","content":"What directory am I in?"}],"tools":[{"type":"function","function":{"name":"bash","description":"Run a command","parameters":{"type":"object","properties":{"command":{"type":"string"}}}}}]}'
   # Response should have tool_calls array
   ```

5. **Anthropic Format Test**
   ```bash
   curl -X POST http://127.0.0.1:6767/v1/messages \
     -H "Content-Type: application/json" \
     -d '{"model":"gpt-4","max_tokens":1000,"messages":[{"role":"user","content":"List files"}],"tools":[{"name":"bash","description":"Run command","input_schema":{"type":"object","properties":{"command":{"type":"string"}}}}]}'
   # Response content should have tool_use block
   ```

6. **Claude Code Integration Test**
   ```bash
   export ANTHROPIC_BASE_URL=http://127.0.0.1:6767
   claude  # Start Claude Code
   # Type: "What is my current working directory?"
   # Should execute pwd and return actual directory
   ```

7. **Streaming Test**
   ```bash
   curl -X POST http://127.0.0.1:6767/v1/chat/completions \
     -H "Content-Type: application/json" \
     -d '{"model":"gpt-4","messages":[{"role":"user","content":"Run ls"}],"tools":[...],"stream":true}'
   # Should receive SSE events with tool_calls in final chunk
   ```

---

## Open Questions

| # | Question | Resolution |
|---|----------|------------|
| 1 | Should we support `tool_choice: "required"`? | Defer to v2; "auto" only for now |
| 2 | How to handle very long tool definitions? | Truncate with warning if > 4000 tokens |
| 3 | Should tool injection be configurable (JSON vs XML)? | Start with JSON only; add flag later if needed |
| 4 | Rate limit impact of longer prompts? | Document in README; monitor in production |

---

## References

| Document | Description |
|----------|-------------|
| [TOOLS-SUPPORT.design.md](./TOOLS-SUPPORT.design.md) | Design document with research |
| [OpenAI Function Calling](https://platform.openai.com/docs/guides/function-calling) | OpenAI tool/function spec |
| [Anthropic Tool Use](https://docs.anthropic.com/en/docs/tool-use) | Anthropic tool use spec |
| [LiteLLM Function Calling](https://docs.litellm.ai/docs/completion/function_call) | Reference implementation |
| [IMPLEMENTATION.plan.md](./IMPLEMENTATION.plan.md) | Main adapter implementation plan |
