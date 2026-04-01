# Native Tools Streaming — Design Document

**Status:** Implemented
**Date:** 2026-03-31
**Severity:** Medium (UX improvement)
**Related:** `TOOLS-SUPPORT.design.md`, `DUAL-RESPONSES.design.md`

---

## Executive Summary

The copilot-adapter currently uses **XML prompt injection** for tool support, which requires buffering the entire response before emitting SSE events. This causes a poor UX where responses appear all at once instead of streaming progressively.

Research shows that **GitHub Copilot supports native OpenAI-style function calling**. Migrating to native tools would enable:
1. Progressive streaming (text + tool calls appear as generated)
2. Better UX matching native Anthropic behavior
3. Simpler, more reliable code (no XML parsing)

This document analyzes the current architecture, compares it with litellm's approach, and proposes a migration path.

---

## Problem Statement

### Observed Behavior

When using the copilot-adapter with Claude Code:
- Response text, tool calls, errors, and follow-up text all appear at once
- No progressive "thinking" visualization
- No incremental tool execution feedback
- User must wait for entire response before seeing anything

### Expected Behavior (Native Anthropic API)

- Text streams token-by-token showing reasoning
- Tool use blocks appear as they are generated
- User can see tools running progressively
- Interactive prompts appear in real-time

### Root Cause

The `handle_streaming_with_tools` function in `src/handlers/messages.rs` buffers ALL chunks until stream completion:

```rust
async fn handle_streaming_with_tools(...) {
    let event_stream = async_stream::stream! {
        let mut buffered_chunks: Vec<...> = Vec::new();
        let mut content_buffer = String::new();

        // PROBLEM: Buffers ALL chunks before processing
        while let Some(result) = stream.next().await {
            content_buffer.push_str(text);   // Accumulate
            buffered_chunks.push(chunk);      // Store
        }

        // Stream ended — NOW parse XML and emit events
        let tool_calls = parser::parse_tool_calls(&content_buffer);
        // ... emit all events at once
    };
}
```

This buffering is necessary because tool calls are embedded as XML in the text response and cannot be parsed until the closing `</function_calls>` tag arrives.

---

## Current Architecture Analysis

### Request Flow (Current)

```
┌─────────────────────────────────────────────────────────────────────────┐
│ Claude Code                                                              │
│ (Anthropic format with tools)                                           │
└─────────────────────────────────────────────────────────────────────────┘
                                    │
                                    ▼
┌─────────────────────────────────────────────────────────────────────────┐
│ copilot-adapter: /v1/messages handler                                    │
│                                                                          │
│  1. Extract tools from request                                          │
│  2. Convert Anthropic tools → internal Tool format                      │
│  3. Inject tools as XML into system prompt                              │
│  4. Remove 'tools' from request params                                  │
│  5. Translate Anthropic → OpenAI format                                 │
└─────────────────────────────────────────────────────────────────────────┘
                                    │
                                    ▼
┌─────────────────────────────────────────────────────────────────────────┐
│ GitHub Copilot API                                                       │
│ (OpenAI format, no native tools — just plain text)                      │
└─────────────────────────────────────────────────────────────────────────┘
                                    │
                                    ▼
┌─────────────────────────────────────────────────────────────────────────┐
│ Response: Plain text with embedded XML                                   │
│                                                                          │
│  "I'll search for that pattern..."                                      │
│  <function_calls>                                                        │
│    <invoke name="Grep">                                                  │
│      <parameter name="pattern">foo</parameter>                          │
│    </invoke>                                                             │
│  </function_calls>                                                       │
│  "Let me also check..."                                                  │
└─────────────────────────────────────────────────────────────────────────┘
                                    │
                                    ▼
┌─────────────────────────────────────────────────────────────────────────┐
│ copilot-adapter: Response processing                                     │
│                                                                          │
│  1. ⚠️ BUFFER entire response (wait for stream end)                     │
│  2. Parse XML tool calls from buffered text                              │
│  3. Strip XML from text content                                          │
│  4. Emit Anthropic SSE events (all at once)                             │
└─────────────────────────────────────────────────────────────────────────┘
                                    │
                                    ▼
┌─────────────────────────────────────────────────────────────────────────┐
│ Claude Code                                                              │
│ (Receives everything at once — poor UX)                                 │
└─────────────────────────────────────────────────────────────────────────┘
```

### Why Buffering Is Required (Current Design)

1. **XML is embedded in text** — Tool calls appear as `<function_calls>` blocks mixed with prose
2. **XML requires complete tags** — Cannot parse `<function_calls><invoke name="Grep">` until closed
3. **Must separate text from tools** — The text block shouldn't contain raw XML markup

---

## Research: litellm's Native Tools Approach

### Key Discovery

litellm's GitHub Copilot integration (`litellm/llms/github_copilot/`) uses a fundamentally different approach:

1. **Translates Anthropic `tools` to OpenAI `tools` format** (native function calling)
2. **Sends native tools to GitHub Copilot** — Copilot returns structured `tool_calls` in response
3. **Streams incrementally** — Each chunk is translated and yielded immediately

### litellm Architecture

```
┌─────────────────────────────────────────────────────────────────────────┐
│ Anthropic Request (with tools)                                          │
└─────────────────────────────────────────────────────────────────────────┘
                                    │
                                    ▼
┌─────────────────────────────────────────────────────────────────────────┐
│ litellm: AnthropicAdapter                                                │
│                                                                          │
│  translate_anthropic_tools_to_openai()                                  │
│  - Converts tool definitions to OpenAI format                           │
│  - Handles tool name truncation (64-char limit)                         │
└─────────────────────────────────────────────────────────────────────────┘
                                    │
                                    ▼
┌─────────────────────────────────────────────────────────────────────────┐
│ GitHub Copilot API                                                       │
│ (OpenAI format WITH native tools parameter)                             │
└─────────────────────────────────────────────────────────────────────────┘
                                    │
                                    ▼
┌─────────────────────────────────────────────────────────────────────────┐
│ Response: Structured OpenAI streaming with tool_calls                    │
│                                                                          │
│  {"delta": {"content": "I'll search..."}}                               │
│  {"delta": {"tool_calls": [{"function": {"name": "Grep"}}]}}            │
│  {"delta": {"tool_calls": [{"function": {"arguments": "{\"pattern\":"}}]}} │
└─────────────────────────────────────────────────────────────────────────┘
                                    │
                                    ▼
┌─────────────────────────────────────────────────────────────────────────┐
│ litellm: AnthropicStreamWrapper                                          │
│                                                                          │
│  FOR EACH chunk:                                                         │
│    - Detect content type (text vs tool_use)                              │
│    - Translate to Anthropic event format                                 │
│    - YIELD immediately                                                   │
└─────────────────────────────────────────────────────────────────────────┘
                                    │
                                    ▼
┌─────────────────────────────────────────────────────────────────────────┐
│ Claude Code                                                              │
│ (Receives progressive streaming — good UX)                              │
└─────────────────────────────────────────────────────────────────────────┘
```

### litellm Code References

**Tool Translation** (`adapters/transformation.py`):
```python
def translate_anthropic_tools_to_openai(
    self, tools: List[AllAnthropicToolsValues], model: Optional[str] = None
) -> Tuple[List[ChatCompletionToolParam], Dict[str, str]]:
    """
    Translate Anthropic tools to OpenAI format.
    Returns: (translated_tools, tool_name_mapping)
    """
```

**Streaming Translation** (`adapters/streaming_iterator.py`):
```python
class AnthropicStreamWrapper:
    async def __anext__(self):
        async for chunk in self.completion_stream:
            # Detect if new content block needed
            should_start_new_block = self._should_start_new_content_block(chunk)

            # Translate OpenAI chunk to Anthropic format
            processed_chunk = translate_streaming_openai_response_to_anthropic(chunk)

            # YIELD immediately - no buffering!
            self.chunk_queue.append(processed_chunk)
            return self.chunk_queue.popleft()
```

**Content Block Detection** (`streaming_iterator.py`):
```python
def _should_start_new_content_block(self, chunk):
    # Detect transition from text to tool_use
    if block_type != self.current_content_block_type:
        self.current_content_block_type = block_type
        return True

    # For parallel tool calls, new name = new block
    if block_type == "tool_use" and tool_block.get("name"):
        return True
```

---

## Proposed Solution: Migrate to Native OpenAI Tools

### Option A: Full Migration (Recommended)

Replace XML prompt injection with native OpenAI tools passthrough:

1. **Pass tools through to Copilot** — Translate Anthropic → OpenAI tool format
2. **Receive structured tool_calls** — Copilot returns OpenAI-format tool calls
3. **Translate incrementally** — Convert each chunk to Anthropic format immediately
4. **No buffering required** — Stream events as they arrive

### Option B: Heuristic-Based XML Streaming (Fallback)

If native tools don't work reliably:

1. Stream text immediately until `<function_calls>` detected
2. Buffer only the XML block
3. Parse and emit tool_use blocks when `</function_calls>` found
4. Resume text streaming

This is more complex and error-prone but doesn't require native tool support.

### Option C: Status Quo (Not Recommended)

Keep current buffering behavior. Document the UX limitation.

---

## Recommended Approach: Option A

### Phase 1: Verify Native Tools Support

Before implementing, verify that GitHub Copilot accepts and returns native tool calls:

```bash
curl -X POST https://api.githubcopilot.com/chat/completions \
  -H "Authorization: Bearer $TOKEN" \
  -H "Content-Type: application/json" \
  -d '{
    "model": "gpt-4",
    "messages": [{"role": "user", "content": "What is the weather in London?"}],
    "tools": [{
      "type": "function",
      "function": {
        "name": "get_weather",
        "description": "Get weather for a location",
        "parameters": {
          "type": "object",
          "properties": {
            "location": {"type": "string"}
          },
          "required": ["location"]
        }
      }
    }]
  }'
```

Expected response includes `tool_calls`:
```json
{
  "choices": [{
    "message": {
      "role": "assistant",
      "tool_calls": [{
        "id": "call_xxx",
        "type": "function",
        "function": {
          "name": "get_weather",
          "arguments": "{\"location\": \"London\"}"
        }
      }]
    }
  }]
}
```

### Phase 2: Implement Tool Translation

**New/Modified Files:**

| File | Changes |
|------|---------|
| `src/tools/translator.rs` | **New**: Anthropic → OpenAI tool translation |
| `src/copilot/types.rs` | Add `tools` parameter to `ChatCompletionRequest` |
| `src/copilot/client.rs` | Pass tools through to Copilot API |
| `src/handlers/messages.rs` | Use native tools path when available |

**Tool Translation** (`src/tools/translator.rs`):

```rust
/// Translate Anthropic tool definitions to OpenAI format.
pub fn translate_anthropic_tools_to_openai(
    tools: &[AnthropicToolDefinition],
) -> (Vec<OpenAITool>, HashMap<String, String>) {
    let mut openai_tools = Vec::new();
    let mut name_mapping = HashMap::new();

    for tool in tools {
        // Handle OpenAI's 64-char name limit
        let truncated_name = truncate_tool_name(&tool.name);
        if truncated_name != tool.name {
            name_mapping.insert(truncated_name.clone(), tool.name.clone());
        }

        openai_tools.push(OpenAITool {
            tool_type: "function".to_string(),
            function: OpenAIFunction {
                name: truncated_name,
                description: tool.description.clone(),
                parameters: translate_input_schema(&tool.input_schema),
            },
        });
    }

    (openai_tools, name_mapping)
}
```

### Phase 3: Implement Incremental Streaming

**Streaming State Machine:**

```rust
struct StreamingToolState {
    current_block_type: ContentBlockType,
    current_block_index: u32,
    tool_name_mapping: HashMap<String, String>,
}

enum ContentBlockType {
    Text,
    ToolUse,
}

impl StreamingToolState {
    fn process_chunk(&mut self, chunk: &OpenAIChunk) -> Vec<AnthropicEvent> {
        let mut events = Vec::new();

        for choice in &chunk.choices {
            // Text content
            if let Some(text) = &choice.delta.content {
                if self.current_block_type != ContentBlockType::Text {
                    events.push(self.emit_content_block_stop());
                    events.push(self.emit_content_block_start_text());
                    self.current_block_type = ContentBlockType::Text;
                }
                events.push(self.emit_text_delta(text));
            }

            // Tool calls
            if let Some(tool_calls) = &choice.delta.tool_calls {
                for tc in tool_calls {
                    if let Some(name) = &tc.function.name {
                        // New tool call starting
                        if self.current_block_type != ContentBlockType::Text {
                            events.push(self.emit_content_block_stop());
                        }
                        events.push(self.emit_content_block_start_tool_use(tc));
                        self.current_block_type = ContentBlockType::ToolUse;
                    }
                    if let Some(args) = &tc.function.arguments {
                        // Tool arguments streaming
                        events.push(self.emit_input_json_delta(args));
                    }
                }
            }
        }

        events
    }
}
```

### Phase 4: Fallback to XML Injection

For models or configurations where native tools don't work:

```rust
pub async fn handle_messages(
    request: AnthropicRequest,
    state: &AppState,
) -> Result<Response, AppError> {
    let has_tools = request.tools.is_some();

    if has_tools && state.config.use_native_tools {
        // Try native tools first
        match handle_with_native_tools(&request, state).await {
            Ok(response) => return Ok(response),
            Err(e) if e.is_tools_not_supported() => {
                tracing::warn!("Native tools not supported, falling back to XML injection");
            }
            Err(e) => return Err(e),
        }
    }

    // Fallback to XML prompt injection (current behavior)
    handle_with_xml_injection(&request, state).await
}
```

---

## Technical Details

### OpenAI Tool Format

**Request:**
```json
{
  "tools": [{
    "type": "function",
    "function": {
      "name": "get_weather",
      "description": "Get weather for a location",
      "parameters": {
        "type": "object",
        "properties": {
          "location": {"type": "string", "description": "City name"}
        },
        "required": ["location"]
      }
    }
  }],
  "tool_choice": "auto"
}
```

**Streaming Response:**
```json
{"choices": [{"delta": {"role": "assistant"}}]}
{"choices": [{"delta": {"content": "I'll check the weather. "}}]}
{"choices": [{"delta": {"tool_calls": [{"index": 0, "id": "call_abc", "type": "function", "function": {"name": "get_weather"}}]}}]}
{"choices": [{"delta": {"tool_calls": [{"index": 0, "function": {"arguments": "{\"location\":"}}]}}]}
{"choices": [{"delta": {"tool_calls": [{"index": 0, "function": {"arguments": " \"London\"}"}}]}}]}
{"choices": [{"finish_reason": "tool_calls"}]}
```

### Anthropic Event Format

**message_start:**
```json
{"type": "message_start", "message": {"id": "msg_xxx", "role": "assistant", "content": []}}
```

**content_block_start (text):**
```json
{"type": "content_block_start", "index": 0, "content_block": {"type": "text", "text": ""}}
```

**content_block_delta (text):**
```json
{"type": "content_block_delta", "index": 0, "delta": {"type": "text_delta", "text": "I'll check"}}
```

**content_block_start (tool_use):**
```json
{"type": "content_block_start", "index": 1, "content_block": {"type": "tool_use", "id": "call_abc", "name": "get_weather", "input": {}}}
```

**content_block_delta (input_json):**
```json
{"type": "content_block_delta", "index": 1, "delta": {"type": "input_json_delta", "partial_json": "{\"location\":"}}
```

### Tool Name Truncation

OpenAI has a 64-character limit for function names. litellm uses:

```rust
const OPENAI_MAX_TOOL_NAME_LENGTH: usize = 64;
const TOOL_NAME_HASH_LENGTH: usize = 8;

fn truncate_tool_name(name: &str) -> String {
    if name.len() <= OPENAI_MAX_TOOL_NAME_LENGTH {
        return name.to_string();
    }

    // Format: {55-char-prefix}_{8-char-hash}
    let hash = sha256(name)[..TOOL_NAME_HASH_LENGTH];
    format!("{}_{}", &name[..55], hash)
}
```

The mapping is stored to restore original names in responses.

---

## Migration Plan

### Step 1: Add Native Tools Infrastructure (Non-Breaking)

- Add tool translation functions
- Add OpenAI tool types
- Add streaming state machine
- Keep XML injection as default

### Step 2: Feature Flag for Native Tools

```bash
copilot-adapter start --native-tools
```

- Enables native tools path
- Falls back to XML on error
- Allows A/B testing

### Step 3: Verify with Claude Code

- Test all common tools (Read, Write, Edit, Bash, Grep, etc.)
- Test streaming UX
- Test error handling
- Test tool results flow

### Step 4: Make Native Tools Default

- If successful, flip default to native tools
- Keep XML injection as fallback option
- Document migration

### Step 5: Deprecate XML Injection (Optional)

- If native tools prove reliable
- Remove XML injection code
- Simplify codebase

---

## Risk Assessment

| Risk | Impact | Probability | Mitigation |
|------|--------|-------------|------------|
| Copilot doesn't support native tools | High | Low | litellm uses it successfully; verify first |
| Tool name truncation issues | Medium | Medium | Use litellm's proven hash-based approach |
| Streaming state machine bugs | Medium | Medium | Comprehensive unit tests |
| Regression in tool calling | High | Low | Feature flag; keep XML fallback |
| Performance impact | Low | Low | Fewer transformations should be faster |

---

## Success Criteria

1. **Progressive streaming** — Text appears token-by-token
2. **Incremental tool visibility** — Tool calls appear as generated
3. **No buffering** — Events emitted immediately
4. **Reliability** — 99%+ tool call success rate
5. **Backward compatible** — XML fallback available

---

## Testing Strategy

### Unit Tests

1. Tool translation (Anthropic → OpenAI)
2. Tool name truncation and mapping
3. Streaming state machine transitions
4. Event emission for all content types
5. Error handling

### Integration Tests

1. Native tools request/response flow
2. Streaming with tool calls
3. Multiple tool calls in one response
4. Tool results handling
5. Fallback to XML injection

### Manual E2E Tests

1. Claude Code with native tools
2. Compare streaming UX: native vs XML
3. Complex multi-tool workflows
4. Error scenarios

---

## References

- `TOOLS-SUPPORT.design.md` — Original XML-based tools implementation
- `DUAL-RESPONSES.design.md` — XML format migration
- [litellm GitHub Copilot source](https://github.com/BerriAI/litellm/tree/main/litellm/llms/github_copilot)
- [litellm AnthropicAdapter source](https://github.com/BerriAI/litellm/tree/main/litellm/llms/anthropic/experimental_pass_through/adapters)
- [OpenAI Function Calling](https://platform.openai.com/docs/guides/function-calling)
- [Anthropic Tool Use](https://docs.anthropic.com/en/docs/tool-use)

---

## Appendix A: Epic 0 Verification Results

**Date:** 2026-03-31

### Summary

Epic 0 verified that the copilot-adapter's type system and client infrastructure
can handle native OpenAI-format tool calls. Testing was done via:

1. **15 unit tests** (`tests/unit/native_tools_verification_tests.rs`) — type serialization/deserialization
2. **5 integration tests** (`tests/integration/native_tools_verification_tests.rs`) — mock server round-trips
3. **Verification scripts** (`scripts/verify-native-tools.sh`, `scripts/verify-native-tools.ps1`) — for manual testing with a live Copilot token

### Key Findings

#### ✅ Finding 1: Type System Supports Native Tool Calls

The existing types in `src/tools/types.rs` and `src/copilot/types.rs` already support the
OpenAI tool call format:

- **Request**: `Tool` / `Function` / `FunctionParameters` serialize correctly to OpenAI format
- **Response**: `ToolCall` / `FunctionCall` deserialize from both streaming and non-streaming responses
- **Streaming**: `ChunkDelta.tool_calls` handles partial deltas (only `arguments` fragment, no `id`/`name`)
- **Request forwarding**: `ChatCompletionRequest.tools` and `.tool_choice` serialize and are included in the JSON body sent to the API

#### ✅ Finding 2: Streaming Tool Call Reconstruction Works

The streaming format uses incremental deltas that can be accumulated:

```
Chunk 1: { delta: { role: "assistant" } }
Chunk 2: { delta: { tool_calls: [{ index: 0, id: "call_001", type: "function", function: { name: "get_weather", arguments: "" } }] } }
Chunk 3: { delta: { tool_calls: [{ index: 0, function: { arguments: "{\"loc" } }] } }
Chunk 4: { delta: { tool_calls: [{ index: 0, function: { arguments: "ation\":\"London\"}" } }] } }
Chunk 5: { delta: {}, finish_reason: "tool_calls" }
```

Accumulating `function.arguments` across chunks produces valid JSON.

#### ✅ Finding 3: Parameter Types Are Preserved

Native tool calls use JSON strings for arguments, which preserve types:

```json
{"query": "test", "limit": 10, "recursive": true}
```

- `limit` is a JSON number (not string `"10"`)
- `recursive` is a JSON boolean (not string `"true"`)

This solves the MCP validation error described in the plan.

#### ⚠️ Finding 4: `MessageContent` Cannot Handle `null` Content (BLOCKER)

When models return native tool calls, they typically set `"content": null` in the
assistant message. The current `MessageContent` type:

```rust
#[serde(untagged)]
pub enum MessageContent {
    Text(String),
    Blocks(Vec<ContentBlock>),
}
```

Cannot deserialize `null` because neither `String` nor `Vec<ContentBlock>` matches.

**Impact**: This must be fixed before native tools can work end-to-end.

**Proposed fix** (for Epic 1/2):
```rust
#[serde(untagged)]
pub enum MessageContent {
    Text(String),
    Blocks(Vec<ContentBlock>),
    Null,  // handles null content from native tool call responses
}
```

Or use `Option<MessageContent>` on the `Message.content` field.

#### ✅ Finding 5: Tool Name Truncation Design Is Sound

The plan proposes truncating tool names longer than 64 chars using:
- 55-char prefix + `_` + 8-char SHA-256 hex hash = 64 chars

The type system (`Tool.function.name`) accepts any string length, so truncation
must be applied in the translation layer (Epic 1). Verification scripts test
64-char, 65-char, and 100-char names empirically against the live API.

### Files Created/Modified

| File | Action | Purpose |
|------|--------|---------|
| `scripts/verify-native-tools.sh` | Created | Bash verification script (4 tests) |
| `scripts/verify-native-tools.ps1` | Created | PowerShell verification script |
| `tests/unit/native_tools_verification_tests.rs` | Created | 15 unit tests |
| `tests/integration/native_tools_verification_tests.rs` | Created | 5 integration tests |
| `tests/common/mock_copilot.rs` | Modified | Added native tool call mock helpers |
| `tests/unit/mod.rs` | Modified | Registered new test module |
| `tests/integration/mod.rs` | Modified | Registered new test module |

### Conclusion

Native tools support via the Copilot API is feasible. The type infrastructure is
largely in place. The main blocker is the `MessageContent` null handling, which is
a small fix. The project should proceed to Epic 1 (Tool Translation Layer).

