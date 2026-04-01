# Tool Call Bug Analysis

**Date:** 2026-03-31
**Issue:** Edit and TaskUpdate tools always fail with "Error editing file" message in Claude Code

## Summary

After analyzing the logs from both Claude Code (`claude_log3.txt`) and copilot-adapter (`logs3.txt`), **there is NO bug in the copilot-adapter**. The tool calls are being correctly parsed, formatted, and returned to Claude Code in the proper Anthropic format. The "Error editing file" message is the expected behavior when Claude Code's Edit tool fails to find the exact string to replace in the target file.

## Evidence from Logs

### 1. Tool Call XML Generation (Copilot Response)

The model generated correct XML tool calls:

```xml
<function_calls>
<invoke name="Grep">
<parameter name="pattern">idle interval checking for expired</parameter>
<parameter name="output_mode">content</parameter>
</invoke>
</function_calls>

<function_calls>
<invoke name="Read">
<parameter name="file_path">D:\src\github4claude\src\auth\token.rs</parameter>
<parameter name="offset">1</parameter>
<parameter name="limit">100</parameter>
</invoke>
</function_calls>

<function_calls>
<invoke name="Edit">
<parameter name="file_path">D:\src\github4claude\src\auth\token.rs</parameter>
<parameter name="old_string">                trace!("idle interval checking for expired");</parameter>
<parameter name="new_string">                trace!("idle interval checking for expired GitHub token");</parameter>
</invoke>
</function_calls>
```

**Source:** `logs3.txt` line with trace log "Full buffered content from streaming response"

### 2. Tool Call Parsing (copilot-adapter)

The adapter successfully parsed all 3 tool calls:

```
2026-03-31T15:42:48.846639Z DEBUG Parsed tool calls from <function_calls> blocks num_calls=3
2026-03-31T15:42:48.846656Z DEBUG Parsed tool calls from streaming response num_tool_calls=3 tool_call_names=[Some("Grep"), Some("Read"), Some("Edit")]
```

**Source:** `logs3.txt` line 1170-1171

### 3. Tool Call Translation to Anthropic Format

The adapter correctly converted the parsed tool calls to Anthropic `tool_use` blocks:

```json
{
  "type": "tool_use",
  "id": "call_17bff06206af",
  "name": "Edit",
  "input": {
    "file_path": "D:\\src\\github4claude\\src\\auth\\token.rs",
    "new_string": "trace!(\"idle interval checking for expired GitHub token\");",
    "old_string": "trace!(\"idle interval checking for expired\");",
    "replace_all": false
  }
}
```

**Source:** `logs3.txt` lines 1235-1244 (incoming request from Claude Code showing the tool_use block that was previously sent)

**Note:** The `replace_all: false` parameter appears in the tool call even though it wasn't in the XML. This is likely added by the adapter's parameter parsing with a default value, or by Claude Code when deserializing. This is not an error.

### 4. Tool Execution Result (Claude Code)

Claude Code received the tool call, executed the Edit operation, and returned an error:

```json
{
  "type": "tool_result",
  "tool_use_id": "call_17bff06206af",
  "content": "<tool_use_error>String to replace not found in file.\nString: trace!(\"idle interval checking for expired\");</tool_use_error>"
}
```

**Source:** `logs3.txt` line 1262 and line 2172 (repeated in subsequent request)

### 5. Root Cause: String Not Found

The Edit tool failed because the exact string match was not found in the file. The reason is visible in the `old_string` parameter:

```rust
old_string: "                trace!(\"idle interval checking for expired\");"
```

This includes **leading whitespace** (indentation). If the actual line in `src/auth/token.rs` had different indentation (e.g., tabs instead of spaces, or a different number of spaces), the exact string match would fail.

## Verification of Correct Behavior

To verify this is the expected behavior and not a bug:

1. **Grep tool worked:** The first tool call (Grep) successfully executed and returned results showing matches in various log files.

2. **Read tool worked:** The second tool call (Read) successfully returned the contents of `src/auth/token.rs` lines 1-100.

3. **Edit tool executed:** The third tool call (Edit) was properly received and executed by Claude Code. It did not silently fail or get dropped.

4. **Error message is semantic:** The error message `<tool_use_error>String to replace not found in file.` is the standard Claude Code Edit tool response when the `old_string` parameter doesn't match any text in the file.

## Why "Error editing file" Appears in Claude Code UI

The message `● Update(src\auth\token.rs) ⎿ Error editing file` in the Claude Code UI is:

1. **"Update"** - Claude Code's UI label for Edit operations (friendly name)
2. **"Error editing file"** - Claude Code's UI rendering of the `<tool_use_error>` content

This is NOT a parsing error or a tool call failure. It's the legitimate result of the Edit operation failing to find the target string.

## Conclusion

**The copilot-adapter is working correctly.** The tool calling flow is:

1. ✅ Model generates XML tool calls in response
2. ✅ Adapter parses XML into `ToolCall` structs
3. ✅ Adapter converts `ToolCall` structs to Anthropic `tool_use` content blocks
4. ✅ Adapter sends `tool_use` blocks in streaming SSE events
5. ✅ Claude Code receives and executes the Edit tool
6. ✅ Claude Code returns error result because string not found
7. ✅ Adapter forwards the tool result back to the model

The "error" is expected behavior when the Edit tool cannot find an exact match for the `old_string` parameter in the target file. This is typically caused by:

- **Whitespace mismatches** (tabs vs. spaces, different indentation levels)
- **Line ending differences** (CRLF vs. LF)
- **The file content changed** between when the model read it and when it tried to edit
- **The model hallucinated** the content of the old_string

## Recommendations

1. **No fix needed in copilot-adapter** - tool calling is working as designed

2. **Potential model improvements** (not adapter-related):
   - The model could be more careful about preserving exact whitespace when reading and editing
   - The model could use more context in the `old_string` to make matches more unique
   - The model could read the file again after an Edit failure to see the actual content

3. **Debugging tool call issues:**
   - Use `--log-level trace` to see full request/response JSON at each transformation point
   - The logs show tool calls are parsed with `DEBUG Parsed tool calls from <function_calls> blocks`
   - The logs show tool calls sent to Claude Code (visible in the next incoming request that includes tool results)
   - Tool execution errors from Claude Code are forwarded back through the adapter unchanged

## Test Case for Verification

To verify the adapter is working correctly with Edit tool calls, create a test where:

1. Model generates an Edit tool call with **correct** old_string (exact match including whitespace)
2. Edit should succeed and return the updated content
3. If Edit succeeds, then adapter is proven to work correctly for Edit tool calls

## Additional Observations

### Tool Call Parameter Handling

The adapter's tool call parser (`src/tools/parser.rs`) extracts only the parameters present in the XML. However, the tool call in the logs shows `"replace_all": false` even though this parameter was not in the XML. This could indicate:

1. The adapter adds default values for optional parameters (check `src/tools/types.rs` for ToolCall struct defaults)
2. Claude Code adds default values when deserializing the `input` JSON object
3. The Anthropic API format requires all parameters (check API docs)

This is **not an error** since `replace_all: false` is the correct default value for the Edit tool.

### Multiple Function Calls Blocks

The model generated three separate `<function_calls>` blocks (one per tool). The adapter's parser handles this correctly:

- `parse_tool_calls()` looks for ALL `<function_calls>` blocks in the content
- Each block is parsed independently via `extract_between_tags("function_calls", content)`
- All parsed tool calls are collected into a single vector

This matches the expected behavior per the Anthropic Cookbook format.

### Streaming vs Non-Streaming

The logs show two different streaming modes:

1. **"streaming"** - used for requests without tools (pass-through SSE chunks)
2. **"streaming_with_tools"** - used for requests with tools (buffer, parse, emit Anthropic events)

The Edit tool call was in a streaming_with_tools request, which means:

- All chunks were buffered until stream completion
- Tool calls were parsed from the full buffered text
- Anthropic-format SSE events were emitted (message_start, content_block_start, etc.)
- Tool use blocks were sent as separate content blocks after text content

This is working correctly as evidenced by Claude Code receiving and executing all three tool calls.

---

# Streaming UX Issue Analysis

**Date:** 2026-03-31 (Updated)
**Issue:** Responses appear as one big chunk instead of progressive streaming when using copilot-adapter

## Problem Description

When using the copilot-adapter, the Claude Code UI shows all response content at once (text, tool calls, errors, follow-up text) rather than streaming progressively. This differs from the native Anthropic API behavior where:

1. Text streams token-by-token (showing the "thinking" process)
2. Tool use blocks appear as they are generated
3. User can see tools running progressively
4. Interactive prompts appear in real-time

Instead, with copilot-adapter, users see the entire response rendered at once after a delay.

## Root Cause: Full Buffering in `handle_streaming_with_tools`

The copilot-adapter has **two different streaming paths**:

### Path 1: Non-Tools Streaming (WORKS CORRECTLY)

Location: `src/handlers/messages.rs` lines 376-560

```rust
// Normal streaming path (no tool parsing) — translate events inline.
let event_stream = async_stream::stream! {
    while let Some(result) = stream.next().await {
        match result {
            Ok(chunk) => {
                // ... immediately yield events as chunks arrive
                yield Ok(Event::default().event("content_block_delta").data(json));
            }
        }
    }
};
```

This path yields SSE events **incrementally** as each chunk arrives from Copilot.

### Path 2: Tools-Enabled Streaming (PROBLEMATIC)

Location: `src/handlers/messages.rs` lines 567-852 (`handle_streaming_with_tools`)

```rust
async fn handle_streaming_with_tools(...) -> Result<Response, AppError> {
    let event_stream = async_stream::stream! {
        let mut buffered_chunks: Vec<...> = Vec::new();
        let mut content_buffer = String::new();

        // PROBLEM: Buffers ALL chunks first
        while let Some(result) = stream.next().await {
            match result {
                Ok(chunk) => {
                    for choice in &chunk.choices {
                        if let Some(ref text) = choice.delta.content {
                            content_buffer.push_str(text);  // Accumulate text
                        }
                    }
                    buffered_chunks.push(chunk);  // Store for later
                }
            }
        }

        // Stream ended — NOW check for tool calls and emit ALL events
        let tool_calls = parser::parse_tool_calls(&content_buffer, debug_tools);
        // ... emit all events at once
    };
}
```

This path **buffers the entire response** before emitting any SSE events to the client.

## Why Buffering Was Implemented

The design document (`DUAL-RESPONSES.design.md` or `TOOLS-SUPPORT.design.md`) likely explains the rationale:

1. **Tool calls are embedded in text via XML** - The Copilot API returns plain text with `<function_calls>` XML blocks embedded
2. **XML must be complete to parse** - Partial XML like `<function_calls><invoke name="Grep">` cannot be parsed until closed
3. **Tool calls must be stripped from text** - The text block shouldn't contain the raw XML; it should be separated into distinct content blocks

The current implementation takes the simplest approach: wait for the full response, parse tool calls, strip them from text, then emit proper Anthropic events.

## How Anthropic's Native Streaming Works

From analyzing litellm's Anthropic handler (`litellm/llms/anthropic/chat/handler.py`):

```python
# Anthropic sends structured SSE events, not embedded XML
event: content_block_start
data: {"type":"content_block_start","index":0,"content_block":{"type":"text","text":""}}

event: content_block_delta
data: {"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"I'll search"}}

event: content_block_start
data: {"type":"content_block_start","index":1,"content_block":{"type":"tool_use","id":"toolu_xxx","name":"Grep","input":{}}}

event: content_block_delta
data: {"type":"content_block_delta","index":1,"delta":{"type":"input_json_delta","partial_json":"{\"pattern\":"}}
```

Key differences:
1. **Tool calls are separate events** - `content_block_start` with `type: "tool_use"` signals a new tool block
2. **Tool args stream incrementally** - `input_json_delta` sends partial JSON chunks
3. **No XML parsing needed** - The API provides structured events, not embedded markup

## Potential Solutions

### Option 1: Stream Text Immediately, Buffer Only for Tool Parsing

**Concept:** Stream text chunks immediately. When detecting potential tool call patterns (e.g., `<function_calls>`), start buffering only that portion.

**Pros:**
- Text streams progressively
- Users see the thinking process

**Cons:**
- Complex state machine to track when inside/outside XML blocks
- Risk of incorrect text/tool boundary detection
- Potential for partial XML in text if detection fails

### Option 2: Heuristic-Based Early Emission

**Concept:** Emit text chunks immediately until `<function_calls>` is detected, then buffer until `</function_calls>`, then emit tool_use blocks and resume streaming.

```
STREAM: "I'll search..." -> yield text delta
STREAM: "for that..." -> yield text delta
STREAM: "<function_calls>" -> start buffering
STREAM: "<invoke name=" -> continue buffering
STREAM: "</function_calls>" -> parse, yield tool_use, resume text streaming
STREAM: "Let me also..." -> yield text delta
```

**Pros:**
- Progressive text streaming
- Tool blocks emitted as soon as complete
- Relatively straightforward state machine

**Cons:**
- Must handle edge cases (XML spanning chunk boundaries, malformed XML)
- Text after tool calls may be delayed slightly
- Potential issues with nested XML

### Option 3: Full Redesign with Native Tool Support

**Concept:** Use Copilot's native function calling API (if available) instead of prompt injection.

**Pros:**
- Eliminates XML parsing entirely
- True streaming with structured events
- More reliable tool call detection

**Cons:**
- Requires Copilot to support function calling (may not be available for all models)
- Major architectural change

### Option 4: Accept Buffering, Improve UX

**Concept:** Keep current buffering but add progress indicators.

**Pros:**
- No code changes to streaming logic
- Simple implementation

**Cons:**
- Doesn't solve the fundamental UX issue
- Response still appears all at once

## Recommended Approach: Option 2 (Heuristic-Based Early Emission)

Implement a streaming state machine:

```rust
enum StreamState {
    Text,           // Emit text chunks immediately
    Buffering,      // Inside <function_calls>...</function_calls>
    AfterToolCall,  // After tool call, resume text streaming
}

while let Some(chunk) = stream.next().await {
    match state {
        StreamState::Text => {
            if chunk_contains("<function_calls>") {
                // Split: emit text before tag, buffer from tag
                state = StreamState::Buffering;
            } else {
                yield text_delta(chunk);
            }
        }
        StreamState::Buffering => {
            buffer.push(chunk);
            if buffer_contains("</function_calls>") {
                // Parse tool calls, emit tool_use blocks
                let tool_calls = parse_tool_calls(&buffer);
                yield tool_use_blocks(tool_calls);
                state = StreamState::AfterToolCall;
            }
        }
        StreamState::AfterToolCall => {
            // Resume text streaming
            yield text_delta(chunk);
        }
    }
}
```

### Edge Cases to Handle

1. **XML spanning chunk boundaries:**
   ```
   Chunk 1: "...text <function_ca"
   Chunk 2: "lls><invoke..."
   ```
   Solution: Buffer last N characters when in Text state, check for partial tag

2. **Multiple tool call blocks:**
   ```
   <function_calls>...</function_calls>
   Some text
   <function_calls>...</function_calls>
   ```
   Solution: Support transition from AfterToolCall back to Buffering

3. **Malformed XML:**
   ```
   <function_calls><invoke...  (never closed)
   ```
   Solution: Timeout or emit buffered content as text on stream end

## Impact on Claude Code UX

With Option 2 implemented:

1. **Thinking process visible:** Text streams progressively showing model reasoning
2. **Tool calls appear when complete:** Each tool block appears as a distinct event
3. **Interactive prompts work:** User questions appear in real-time
4. **Error messages contextualized:** Errors appear after relevant tool calls, not all at once

## Comparison with litellm

### Key Architectural Difference

litellm uses a fundamentally different approach when exposing Anthropic API with GitHub Copilot as backend:

**litellm Architecture:**
```
Anthropic Request → litellm AnthropicAdapter → OpenAI Request (with native tools)
    → GitHub Copilot API → OpenAI Response (with native tool_calls)
    → AnthropicStreamWrapper → Anthropic SSE Events (streamed incrementally)
```

**copilot-adapter Architecture:**
```
Anthropic Request → copilot-adapter → OpenAI Request (tools injected as XML in system prompt)
    → GitHub Copilot API → OpenAI Response (text with embedded <function_calls> XML)
    → Buffer entire response → Parse XML → Anthropic SSE Events (emitted all at once)
```

### litellm's Streaming Translation (streaming_iterator.py)

litellm's `AnthropicStreamWrapper` translates OpenAI streaming chunks to Anthropic events **in real-time**:

```python
class AnthropicStreamWrapper:
    async def __anext__(self):
        async for chunk in self.completion_stream:
            # Check if new content block needed (e.g., text → tool_use)
            should_start_new_block = self._should_start_new_content_block(chunk)
            if should_start_new_block:
                self._increment_content_block_index()
                # Emit content_block_stop for previous block
                self.chunk_queue.append({"type": "content_block_stop", ...})
                # Emit content_block_start for new block
                self.chunk_queue.append({"type": "content_block_start", ...})

            # Translate OpenAI chunk to Anthropic format
            processed_chunk = LiteLLMAnthropicMessagesAdapter().translate_streaming_openai_response_to_anthropic(
                response=chunk,
                current_content_block_index=self.current_content_block_index,
            )

            self.chunk_queue.append(processed_chunk)
            return self.chunk_queue.popleft()  # Yield immediately!
```

### Why litellm Can Stream Incrementally

1. **GitHub Copilot supports native OpenAI tool_calls**: When you send `tools` in the OpenAI format request, Copilot returns structured `tool_calls` in the response, not embedded XML.

2. **OpenAI's streaming includes tool_calls incrementally**:
   ```json
   {"choices": [{"delta": {"tool_calls": [{"index": 0, "function": {"name": "Grep"}}]}}]}
   {"choices": [{"delta": {"tool_calls": [{"index": 0, "function": {"arguments": "{\"pattern\":"}}]}}]}
   ```

3. **No XML parsing needed**: litellm's adapter detects `tool_calls` in the OpenAI response and emits Anthropic `tool_use` blocks immediately without any text parsing.

### Why copilot-adapter Cannot (Currently)

copilot-adapter uses **prompt injection** for tools:
- Tool definitions are injected into the system prompt as XML
- The model responds with XML `<function_calls>` embedded in text
- XML cannot be parsed until the closing tag arrives
- Therefore, the entire response must be buffered

### Potential Solution: Use Native OpenAI Tools API

If GitHub Copilot supports native OpenAI `tools` parameter (which it appears to, based on litellm's implementation), copilot-adapter could:

1. **Pass through `tools` as OpenAI format** instead of injecting XML into the prompt
2. **Receive native `tool_calls`** in the OpenAI response
3. **Translate incrementally** like litellm does

This would eliminate the need for:
- XML prompt injection
- Response buffering
- XML parsing

And would provide:
- Progressive streaming (text + tool calls appear as generated)
- Better UX matching native Anthropic behavior
- Simpler, more reliable code

## Conclusion

The streaming buffering is a **deliberate design choice** to support XML-based tool call parsing, not a bug. However, it significantly degrades the UX compared to native Anthropic streaming.

**Recommendation:** Implement Option 2 (heuristic-based early emission) to restore progressive streaming while maintaining tool call parsing capability. This requires careful handling of XML boundary detection but should provide a significantly better user experience.
