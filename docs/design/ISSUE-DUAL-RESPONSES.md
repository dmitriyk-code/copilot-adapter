# Issue: Dual Model Responses with Context Loss

## Date Discovered
2026-03-30

## Summary
When Claude Code sent a single request ("Let's carefully implement that") through the copilot-adapter, **two different models responded simultaneously** with different behaviors:
1. **Haiku** - responded with NO context (didn't know what "that" referred to)
2. **Sonnet** - responded with full context but failed to generate proper tool calls (wrote markdown code block instead)

## Evidence
See `logs.txt` file with trace-level logging from the adapter.

## Detailed Timeline (from test.logs)

### T=19:14:46.277 - Single Request Sent
- **Endpoint**: `/v1/messages` (Anthropic format)
- **Model requested**: `claude-sonnet-4.5`
- **Message**: "Let's carefully implement that"
- **Full conversation history included** with context about implementing error logging in `src/tools/parser.rs`
- **Request ID**: `59631463-578d-49c9-b75a-11597d5ce20b`

### T=19:14:47.039 - First Response Starts (Haiku)
- **Response ID**: `msg_vrtx_01LqnzrfFzP6oX88bchmhx63`
- **Model**: `claude-haiku-4.5` ⚠️ **WRONG MODEL**
- **Request ID**: `ae24226f-512d-48cd-8f71-060aa2b4adfb` ⚠️ **DIFFERENT REQUEST ID**
- **Mode**: `streaming` (NOT `streaming_with_tools`)
- **Content**:
  ```
  # Let's Implement This Carefully

  I'd be happy to help implement something carefully, but I need more context!

  Could you please clarify:

  1. **What** are we implementing? (e.g., an algorithm, a feature, a system, a design pattern)
  2. **What technology/language** are we using? (Python, JavaScript, Java, etc.)
  3. **What problem** are we solving?
  4. **What "that" are...
  ```
- **Problem**: Complete context loss - doesn't know what "that" refers to
- **Finished at**: T=19:14:48.885 with `finish_reason: "stop"`

### T=19:14:48.999 - Second Response Starts (Sonnet)
- **Response ID**: `msg_01KLDZdr5QuhEcxQCXsGBiBF`
- **Model**: `claude-sonnet-4.5` ✓ **CORRECT MODEL**
- **Request ID**: `59631463-578d-49c9-b75a-11597d5ce20b` ✓ **MATCHES ORIGINAL REQUEST**
- **Mode**: `streaming_with_tools` ✓ **CORRECT MODE**
- **Content**:
  ```
  I'll implement comprehensive error logging for tool call parsing failures. Let me start by reading the full parser file to understand all the parsing paths:

  <answer>Looking at the code, I need to add error logging in three key functions:

  1. **`try_parse_tool_call()`** - JSON tool call parsing
  2. **`try_parse_xml_invoke()`** - XML tool call parsing
  3. **`parse_xml_tool_calls()`** - XML invoke name validation

  Let me implement this carefully:

  ```rust
  Edit src/tools/parser.rs
  ```

  I'll add detailed logging for each failure point while preserving the existing graceful-degradation behavior (silently skip malformed calls, but log why they were skipped).

  </answer>
  ```
- **Problem**: Model has full context but wrote `Edit src/tools/parser.rs` as markdown instead of generating proper tool call JSON/XML
- **Finished at**: T=19:14:52.096 with `finish_reason: "stop"`
- **Tool parsing result**: "No tool calls found in streaming response" (line 146)

## Key Observations

### 1. Two Simultaneous API Calls
The adapter made **two concurrent requests** to GitHub Copilot API:
- Request `59631463-578d-49c9-b75a-11597d5ce20b` → `claude-sonnet-4.5` (correct)
- Request `ae24226f-512d-48cd-8f71-060aa2b4adfb` → `claude-haiku-4.5` (unexpected)

### 2. Different Streaming Modes
- **Haiku request**: `mode="streaming"` (no tool parsing)
- **Sonnet request**: `mode="streaming_with_tools"` (tool parsing enabled)

### 3. Context Loss in Haiku
The Haiku model's response indicates it received the prompt "Let's carefully implement that" **without** the preceding conversation history. The model is asking for clarification about what "that" refers to.

### 4. Tool Call Failure in Sonnet
The Sonnet model clearly understood the task (implementing error logging in `src/tools/parser.rs`) but failed to generate a proper tool call. Instead of:
- XML: `<invoke name="Edit">...</invoke>`
- JSON: `{"function_call": {"name": "Edit", "arguments": {...}}}`

It generated:
```rust
Edit src/tools/parser.rs
```

This suggests the tool injection prompt may not be working correctly, or the model is confused about how to format tool calls.

### 5. Parser Didn't Find Tool Calls
Log line 146: `"No tool calls found in streaming response"`

The adapter's tool parser (`src/tools/parser.rs`) failed to extract any tool calls from the Sonnet response, confirming that the format was invalid.

## Root Causes to Investigate

### Priority 1: Why Two Models?
**File**: `src/handlers/messages.rs` or `src/handlers/chat.rs`
**Question**: What code path causes two simultaneous API requests to different models?
**Hypothesis**:
- Race condition in request handling?
- Duplicate request processing?
- Model fallback logic triggering incorrectly?
- Connection pooling issue causing request duplication?

### Priority 2: Why Context Loss?
**File**: `src/anthropic/types.rs`, `src/handlers/messages.rs`
**Question**: Why did the Haiku request not include conversation history?
**Hypothesis**:
- Message history serialization bug
- Request cloning losing data
- Different code path for fallback model

### Priority 3: Why Tool Call Format Wrong?
**Files**: `src/tools/injector.rs`, `src/tools/parser.rs`
**Question**: Why did Sonnet generate markdown instead of proper tool calls?
**Hypothesis**:
- Tool injection prompt not being added to system message
- Incorrect prompt format for Copilot API
- Model confusion about expected format
- Tool definitions not properly serialized

### Priority 4: Why Different Streaming Modes?
**File**: `src/copilot/client.rs`
**Question**: Why `streaming` for Haiku vs `streaming_with_tools` for Sonnet?
**Hypothesis**:
- Different code paths for the two requests
- Conditional logic based on model type
- Tool parsing disabled for fallback requests

## Relevant Code Files

### Request Handling
- `src/handlers/messages.rs` - Anthropic `/v1/messages` endpoint handler
- `src/handlers/chat.rs` - OpenAI `/v1/chat/completions` endpoint handler
- `src/anthropic/types.rs` - Anthropic request/response type definitions and translation

### Tool Support
- `src/tools/injector.rs` - Tool definition injection into system prompt
- `src/tools/parser.rs` - Tool call parsing from text responses
- `src/tools/types.rs` - Tool/ToolCall type definitions

### Copilot Client
- `src/copilot/client.rs` - HTTP client for GitHub Copilot API with SSE streaming
- `src/copilot/types.rs` - OpenAI request/response types

### Model Handling
- `src/model_mapper.rs` - Model name normalization (Claude Code format → Copilot format)

## Debugging Steps

### Step 1: Trace Request Flow
```bash
# Look for where two API calls originate
grep -n "Initiating streaming request" test.logs
grep -n "request_id=" test.logs | grep -E "(59631463|ae24226f)"
```

### Step 2: Check Model Mapping
```bash
# Find where model selection happens
grep -n "claude-haiku-4.5" test.logs
grep -n "claude-sonnet-4.5" test.logs
```

### Step 3: Examine Tool Injection
```bash
# Check if tools were injected properly
grep -n "Injecting.*tools" test.logs
grep -n "Available Functions" test.logs
```

### Step 4: Review Message History
```bash
# Check if conversation history was included
grep -n '"messages":' test.logs
# Look for the specific user message
grep -n "Let's carefully implement" test.logs
```

## Expected Behavior

When Claude Code sends a single request:
1. **ONE** API call to GitHub Copilot with the requested model (`claude-sonnet-4.5`)
2. Full conversation history included in the request
3. Tool definitions injected into system prompt
4. Model generates response with proper tool call format (XML or JSON)
5. Adapter's parser extracts tool calls and returns them in appropriate format
6. **ONE** response returned to Claude Code

## Actual Behavior

1. ❌ **TWO** API calls made (Sonnet + Haiku)
2. ❌ Haiku request missing conversation history
3. ✓ Tool definitions appear to be injected (need to verify from logs)
4. ❌ Sonnet generates markdown code block instead of tool call
5. ❌ Parser finds no tool calls
6. ❌ **TWO** responses returned to Claude Code (user sees both)

## Impact

This bug affects:
- **User experience**: Confusing to receive two different responses
- **Reliability**: One response is always wrong (missing context)
- **Tool execution**: Sonnet's response doesn't trigger the Edit tool
- **Performance**: Wasted API calls and latency

## Next Steps

1. **Read relevant source files** to understand request flow
2. **Identify** where the second (Haiku) request originates
3. **Determine** why conversation history is not included in Haiku request
4. **Investigate** why tool call format is incorrect in Sonnet response
5. **Fix** the root causes
6. **Test** with the same scenario to verify fix

## Test Scenario to Reproduce

1. Start adapter with trace logging: `copilot-adapter start --log-level trace`
2. In Claude Code, have a conversation about implementing something (build up context)
3. Send the message: "Let's carefully implement that"
4. Observe: Two responses appear simultaneously
5. Check logs: Two different models responding with different request IDs

## Related Documentation

- **CLAUDE.md** - Project overview and architecture
- **DESIGN.md** - Full design document
- **TOOLS-SUPPORT.design.md** - Tool/function support design
- **docs/e2e-testing.md** - Manual testing procedures
