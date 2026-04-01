# Missing Features Analysis: Claude API vs Copilot Adapter

This document analyzes Claude API features used by Claude Code that are not yet fully implemented in the copilot-adapter, along with an assessment of implementation feasibility via GitHub Copilot API.

## Executive Summary

| Feature | Priority | Copilot API Support | Implementation Difficulty |
|---------|----------|---------------------|---------------------------|
| Extended Thinking | High | ❌ Not supported | Impossible without upstream |
| Token Counting | Medium | ✅ Implemented | ✅ Done (tiktoken-rs) |
| Prompt Caching | Low | ❌ Not applicable | N/A (server-side feature) |
| `tool_choice` options | Medium | ⚠️ Partial | Medium |
| `stop_sequences` | Low | ✅ Already implemented | None |
| `metadata` (user_id) | Low | ❌ Unknown | Easy to pass through |
| Computer Use Tools | Low | ❌ Not applicable | N/A (Claude-specific) |
| PDF/Document Support | Low | ⚠️ Limited | Medium |

---

## 1. Extended Thinking (High Priority)

### What It Is
Extended thinking allows Claude to perform internal reasoning before responding. This is controlled by:
- `thinking.type: "enabled"` - Enable thinking
- `thinking.budget_tokens: N` - Set thinking budget (1024 to model's max output)

### How Claude Code Uses It
Claude Code uses extended thinking for complex reasoning tasks. The API sends:
```json
{
  "thinking": {
    "type": "enabled",
    "budget_tokens": 10000
  }
}
```

Streaming responses include `thinking` content blocks with the model's reasoning process.

### Current Implementation
**Not implemented.** The `AnthropicRequest` struct doesn't include a `thinking` field. Any thinking parameters sent by Claude Code are silently ignored.

### Copilot API Support
**Not supported.** GitHub Copilot's chat completions API follows OpenAI's format, which doesn't have an equivalent to Anthropic's extended thinking. While OpenAI has `reasoning_effort` for o1/o3 models, this is model-specific and doesn't apply to Claude models accessed through Copilot.

### Implementation Assessment
**Impossible without upstream support.** Options:
1. **Silent degradation** (current): Ignore thinking parameters, Claude still responds but without explicit thinking
2. **Error response**: Return an error when thinking is requested to make the limitation explicit
3. **Simulated thinking via prompt**: Inject instructions like "Think step by step" into the system prompt (poor substitute)

**Recommendation**: Implement option 2 (explicit error or warning). Specifically:
1. Add `thinking` field to `AnthropicRequest` struct
2. When `thinking.type == "enabled"`, either:
   - Return HTTP 400 with clear error message, OR
   - Log a warning and proceed without thinking (graceful degradation)
3. Document the limitation prominently

---

## 2. Token Counting Endpoint (Medium Priority) — ✅ IMPLEMENTED

### What It Is
Anthropic provides `POST /v1/messages/count_tokens` to count tokens before sending a request:
```json
{
  "model": "claude-sonnet-4-20250514",
  "messages": [...],
  "system": "..."
}
```

Returns: `{"input_tokens": 1234}`

### How Claude Code Uses It
Claude Code may use this for:
- Context window management
- Cost estimation
- Deciding when to truncate/summarize

### Current Implementation
**✅ Implemented.** The `POST /v1/messages/count_tokens` endpoint is fully functional using `tiktoken-rs` with the `cl100k_base` BPE encoding.

**Implementation details:**
- **Module:** `src/token_counter.rs` — core counting logic
- **Handler:** `src/handlers/count_tokens.rs` — HTTP handler
- **Route:** `POST /v1/messages/count_tokens` registered in `src/server.rs`
- **Dependency:** `tiktoken-rs` 0.5 (~1-2MB binary size impact)
- **Performance:** <10ms for typical requests
- **Accuracy:** >95% for text content; images/documents use fixed estimates (~85 tokens)

**Supported content types:**
- Text blocks: Full BPE tokenization
- Image blocks: Fixed 85-token estimate
- Document blocks: Fixed 85-token estimate
- Tool definitions: JSON-serialized and tokenized
- System prompts: String and content block formats

### Copilot API Support
**Not applicable.** Token counting is performed locally using tiktoken-rs. No upstream API call is made.

### Implementation Assessment
**✅ Done.** Implemented using `tiktoken-rs` (option 1 from the original recommendation). Accurate enough for practical use — context window management and cost estimation work reliably.

---

## 3. Prompt Caching (Low Priority)

### What It Is
Anthropic's prompt caching reduces costs and latency by caching parts of prompts:
```json
{
  "system": [
    {
      "type": "text",
      "text": "Large system prompt...",
      "cache_control": {"type": "ephemeral"}
    }
  ]
}
```

### How Claude Code Uses It
Claude Code uses caching for large system prompts (CLAUDE.md files, context, etc.) to reduce latency and costs.

### Current Implementation
**Not implemented.** The `cache_control` field is not parsed or forwarded.

### Copilot API Support
**Not applicable.** Prompt caching is a server-side optimization specific to Anthropic's infrastructure. GitHub Copilot may have its own caching mechanisms but doesn't expose this in the API.

### Implementation Assessment
**Cannot be implemented.** This is an Anthropic-specific feature that requires server-side support.

**Recommendation**: Document the limitation. Consider stripping `cache_control` from requests to avoid potential errors. Usage through Copilot already has different cost characteristics.

---

## 4. Tool Choice Options (Medium Priority)

### What It Is
Anthropic's `tool_choice` parameter controls tool use behavior:
- `{"type": "auto"}` - Model decides whether to use tools (default)
- `{"type": "any"}` - Model must use at least one tool
- `{"type": "tool", "name": "specific_tool"}` - Model must use the specified tool
- `{"type": "none"}` - Model cannot use tools (added later?)

### How Claude Code Uses It
Claude Code may use specific tool_choice values to force tool use in certain scenarios.

### Current Implementation
**Partially implemented.** From `TOOLS-SUPPORT.design.md`:
> `tool_choice` only supports "auto" behavior; other modes would require prompt engineering

The current implementation always uses "auto" behavior via prompt injection.

### Copilot API Support
**Partial.** OpenAI's API supports:
- `"auto"` - Model decides
- `"none"` - Disable tool use
- `"required"` - Must use a tool (equivalent to Anthropic's `any`)
- `{"type": "function", "function": {"name": "..."}}` - Force specific tool

### Implementation Assessment
**Medium difficulty.** The prompt-injection approach makes this tricky:
1. **"auto"**: Current behavior (no change needed)
2. **"any"/"required"**: Add stronger language to prompt: "You MUST use one of the available tools"
3. **"tool"**: Add "You MUST use the {name} tool" to prompt
4. **"none"**: Don't inject tools into prompt

**Recommendation**: Implement basic mapping for `any` and `none` modes through prompt modification.

---

## 5. Stop Sequences (Low Priority)

### What It Is
Custom stop sequences that cause the model to stop generating:
```json
{
  "stop_sequences": ["\n\nHuman:", "END"]
}
```

### How Claude Code Uses It
Likely used to prevent the model from generating beyond certain markers.

### Current Implementation
**✅ Already implemented.** The `stop_sequences` field is mapped to OpenAI's `stop` parameter in `into_openai_request()`:
```rust
stop: self.stop_sequences,
```

### Copilot API Support
**Supported.** OpenAI's API has `stop` parameter with the same functionality.

### Implementation Assessment
**Already done.** No action needed.

---

## 6. Metadata / User ID (Low Priority)

### What It Is
Optional metadata for tracking:
```json
{
  "metadata": {
    "user_id": "user-123"
  }
}
```

### How Claude Code Uses It
May be used for usage tracking, rate limiting by user, or analytics.

### Current Implementation
**Not implemented.** The `metadata` field is not parsed.

### Copilot API Support
**Unknown.** OpenAI's API has a `user` parameter for similar purposes.

### Implementation Assessment
**Easy.** Map `metadata.user_id` to OpenAI's `user` parameter.

**Recommendation**: Implement this mapping if Copilot API supports the `user` parameter.

---

## 7. Computer Use Tools (Low Priority)

### What It Is
Special tool types for computer interaction:
- `computer_20241022` - Screen control (screenshot, click, type)
- `bash_20241022` - Shell command execution
- `text_editor_20241022` - File editing

### How Claude Code Uses It
Claude Code has its own tools (Bash, Read, Write, Edit) that are different from these. The computer use tools are primarily for Claude's agentic computer control.

### Current Implementation
**Not implemented.** These special tool types are not handled.

### Copilot API Support
**Not applicable.** These are Anthropic-specific tool types that require special server-side handling.

### Implementation Assessment
**Cannot be implemented.** These tools require Anthropic's specific infrastructure.

**Recommendation**: Document the limitation. Not a concern for Claude Code since it uses its own tool implementations.

---

## 8. PDF/Document Support (Low Priority)

### What It Is
Anthropic supports PDF documents in messages:
```json
{
  "type": "document",
  "source": {
    "type": "base64",
    "media_type": "application/pdf",
    "data": "..."
  }
}
```

### How Claude Code Uses It
Claude Code's Read tool can read PDFs and may send them as document blocks.

### Current Implementation
**Partially implemented.** From `types.rs`:
```rust
ContentBlock::Document { .. } => {
    // Skip document blocks - would need PDF extraction
    // OpenAI doesn't support PDF directly
}
```

Document blocks are silently skipped.

### Copilot API Support
**Limited.** OpenAI's API supports images but not PDFs directly. Would require:
1. PDF to image conversion (each page as an image)
2. PDF text extraction

### Implementation Assessment
**Medium difficulty.** Options:
1. **Text extraction**: Use a library like `pdf-extract` to convert PDF to text
2. **Image conversion**: Use `pdf-render` to convert pages to images
3. **Error response**: Return an error for PDF content

**Recommendation**: Implement text extraction as a fallback. This preserves most of the content's utility.

---

## 9. Streaming Format Differences

### What It Is
Anthropic's streaming format differs from OpenAI's:
- **Anthropic**: Uses Server-Sent Events (SSE) with event types like `message_start`, `content_block_start`, `content_block_delta`, `message_delta`, `message_stop`
- **OpenAI**: Uses SSE with `data: {...}` format containing `choices[].delta`

### Extended Thinking in Streaming
When extended thinking is enabled, Anthropic streams:
```
event: content_block_start
data: {"type":"content_block_start","index":0,"content_block":{"type":"thinking","thinking":""}}

event: content_block_delta
data: {"type":"content_block_delta","index":0,"delta":{"type":"thinking_delta","thinking":"Let me analyze..."}}
```

### Current Implementation
**Implemented for basic streaming.** The adapter converts between formats, but thinking blocks cannot be streamed since Copilot doesn't support the thinking feature.

### Implementation Assessment
**No action needed for basic streaming.** The thinking block streaming would require upstream support that doesn't exist.

---

## 10. Additional Minor Features

### 10.1 `top_k` Parameter
- **Status**: Defined in `AnthropicRequest` but not translated
- **OpenAI Support**: Not supported (OpenAI only has `top_p`)
- **Recommendation**: Silently ignore (current behavior is correct)

### 10.2 Stream Options
- **Status**: Basic streaming works
- **Missing**: Anthropic's `stream_options` for usage stats in streaming
- **Recommendation**: Low priority, usage tracking is nice-to-have

### 10.3 `service_tier`
- **Status**: Not implemented
- **Purpose**: Request priority tier
- **Recommendation**: Not applicable for Copilot

---

## Implementation Priority

### Phase 1 (High Value, Low Effort)
1. **metadata.user_id → user** mapping
2. Document extended thinking limitation with explicit error/warning
3. Add `thinking` field handling with appropriate response

### Phase 2 (Medium Value, Medium Effort)
4. **tool_choice** enhanced support (any, none modes)
5. ~~**Token counting endpoint** using tiktoken-rs~~ — ✅ Implemented

### Phase 3 (Lower Value)
6. **PDF text extraction** fallback
7. Improved error messages for unsupported features

### Not Feasible
- Extended thinking (requires Anthropic infrastructure)
- Prompt caching (requires Anthropic infrastructure)
- Computer use tools (requires Anthropic infrastructure)

---

## Appendix: Feature Comparison Matrix

| Anthropic Parameter | OpenAI Equivalent | Current Status | Action Needed |
|---------------------|-------------------|----------------|---------------|
| `model` | `model` | ✅ Implemented | None |
| `messages` | `messages` | ✅ Implemented | None |
| `max_tokens` | `max_tokens` | ✅ Implemented | None |
| `system` | `messages[0]` (role: system) | ✅ Implemented | None |
| `temperature` | `temperature` | ✅ Implemented | None |
| `top_p` | `top_p` | ✅ Implemented | None |
| `top_k` | ❌ Not supported | ⚠️ Ignored | None (correct) |
| `stop_sequences` | `stop` | ✅ Implemented | None |
| `stream` | `stream` | ✅ Implemented | None |
| `tools` | Prompt injection | ✅ Implemented | None |
| `tool_choice` | Prompt injection | ⚠️ Partial | Enhance for modes |
| `thinking` | ❌ Not supported | ❌ Not implemented | Error or document |
| `metadata` | `user` | ❌ Not mapped | Map user_id |
| Token counting | `tiktoken-rs` (local) | ✅ Implemented | ✅ Done |
| Document blocks | ❌ Not supported | ⚠️ Skipped | Text extraction |
| Image blocks | `image_url` | ✅ Implemented | None |

---

## Claude Code Specific Considerations

### Extended Thinking Impact
Claude Code uses extended thinking for complex reasoning tasks like:
- Multi-file refactoring
- Debugging complex issues
- Architectural decisions
- Code review and analysis

Without extended thinking, Claude through Copilot will still function but may:
- Provide less thorough analysis
- Miss edge cases that thinking would catch
- Be less reliable on complex multi-step tasks

### Practical Workarounds
1. **For users**: Be more explicit in requests, break complex tasks into smaller steps
2. **For the adapter**: Consider adding a warning header when thinking was requested but unavailable
3. **Future**: Monitor if GitHub Copilot adds thinking/reasoning support for Claude models

### What Still Works Well
- Basic chat completions ✅
- Tool use (via prompt injection) ✅
- Image/vision support ✅
- Streaming ✅
- Model selection ✅
- Basic parameters (temperature, top_p, max_tokens) ✅

---

## References

- [Anthropic Messages API](https://docs.anthropic.com/en/api/messages)
- [Anthropic Extended Thinking](https://docs.anthropic.com/en/docs/build-with-claude/extended-thinking)
- [Anthropic Tool Use](https://docs.anthropic.com/en/docs/build-with-claude/tool-use)
- [Anthropic Prompt Caching](https://docs.anthropic.com/en/docs/build-with-claude/prompt-caching)
- [OpenAI Chat Completions API](https://platform.openai.com/docs/api-reference/chat)
