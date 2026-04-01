# Debugging Tool Call Issues

This guide helps you diagnose why web search, web fetch, or other tool calls might not be working through the copilot-adapter.

## Quick Start

The debug scripts will automatically:
1. ✅ Build the adapter in release mode
2. ✅ Check/perform authentication
3. ✅ Start with trace-level logging

### Linux/macOS:
```bash
chmod +x scripts/debug-responses.sh
./scripts/debug-responses.sh
```

### Windows:
```cmd
scripts\debug-responses.bat
```

This will start the adapter with **trace-level logging** and capture everything to a timestamped log file.

## What Gets Logged

With the enhanced logging, you'll see:

### 1. Request Information (DEBUG level)
```
DEBUG Received Anthropic messages request model="claude-3.5-sonnet" stream=Some(true) num_messages=3 max_tokens=4096
```

### 2. Tool Injection (DEBUG level)
```
DEBUG Injecting Anthropic tools into prompt num_tools=3 tool_names=["WebSearch", "WebFetch", "Bash"]
```

### 3. Raw Response from Copilot (DEBUG level)
```
DEBUG Raw response from Copilot (Anthropic endpoint)
  choice_index=0
  content_length=342
  content_preview="I'll search for information about that topic.\n\n```json\n{\"function_call\": {\"name\": \"WebSearch\", \"arguments\": {\"query\": \"..."
  finish_reason=Some("stop")
  existing_tool_calls=None
```

**Key fields to check:**
- `content_preview` - First 200 chars of the response (look for tool call patterns)
- `existing_tool_calls` - If Copilot natively returned tool_calls (should be `None` for GitHub Copilot)
- `finish_reason` - How the model stopped (`stop`, `length`, etc.)

### 4. Full Response Content (TRACE level)
```
TRACE Full content text from Copilot response
  choice_index=0
  full_content="[entire response text here]"
```

### 5. Full JSON Response (TRACE level)
```
TRACE Full response JSON from Copilot
  response_json="{
    \"id\": \"chatcmpl-...\",
    \"object\": \"chat.completion\",
    \"created\": 1234567890,
    \"model\": \"gpt-4\",
    \"choices\": [
      {
        \"index\": 0,
        \"message\": {
          \"role\": \"assistant\",
          \"content\": \"...full content here...\"
        },
        \"finish_reason\": \"stop\"
      }
    ]
  }"
```

### 6. Tool Call Format Detection (DEBUG level)
```
DEBUG Parsed tool calls from JSON format num_calls=1
```
or, when Claude responds with its native XML format:
```
DEBUG Parsed tool calls from XML format num_calls=1
```

### 7. Parsed Tool Calls (DEBUG level)
```
DEBUG Parsed tool calls from Anthropic response
  num_tool_calls=1
  tool_call_names=["WebSearch"]
```

## Manual Logging Options

### Debug level only (less verbose):
```bash
copilot-adapter start --log-level debug --log-file debug.log
```

### Trace level (very verbose, includes full JSON):
```bash
copilot-adapter start --log-level trace --log-file trace.log
```

### Console output only (no file):
```bash
copilot-adapter start --log-level trace
```

## Analyzing the Logs

### 1. Check if tools are being sent
Search for:
```
Injecting tools into prompt
```

If you don't see this, Claude Code isn't sending tools, or the adapter isn't receiving them.

### 2. Check the raw response format
Look at the `content_preview` or `full_content` fields:

**Expected formats our parser recognizes:**

**JSON format (fenced code block):**
```json
{
  "function_call": {
    "name": "WebSearch",
    "arguments": {"query": "some query"}
  }
}
```

**JSON format (inline):**
```
{"function_call": {"name": "WebSearch", "arguments": {"query": "some query"}}}
```

**XML format (Claude's native tool call style):**
```xml
<function_calls>
  <invoke name="WebSearch">
    <parameter name="query">some query</parameter>
  </invoke>
</function_calls>
```

The parser tries JSON first; if no JSON tool calls are found, it falls back to
XML. Claude models sometimes generate this XML format instead of the requested
JSON format. Multiple `<invoke>` blocks inside a single `<function_calls>` are
supported.

If Copilot's response uses a format other than the above (e.g., plain text tool
names), our parser won't detect it.

### 3. Check if tool calls are being parsed
Search for:
```
Parsed tool calls from
```

If you see tool injection but no parsed tool calls, the model either:
- Didn't generate a tool call
- Generated it in a format our parser doesn't recognize

### 4. Check existing_tool_calls field
```
existing_tool_calls=Some([...])
```

If this is **not `None`**, it means Copilot natively returned tool calls in the response! This would be surprising and indicates GitHub Copilot might have added native tool support.

## Common Issues

### Issue: Tools injected but never parsed
**Symptom:**
```
DEBUG Injecting tools into prompt num_tools=3 ...
DEBUG Raw response ... content_preview="Here's the information you requested: [regular text]"
```

**Cause:** The model didn't generate a tool call. Copilot's GPT-4 might not understand the injected tool format.

**Solutions:**
- Try different prompt formats (future feature: `--tool-prompt-format xml`)
- Check if the query actually needs a tool (model might answer from knowledge)
- See Epic 9 in TOOLS-SUPPORT.plan.md for reliability improvements

### Issue: Tool call in response but not parsed
**Symptom:**
```
DEBUG Raw response ... content_preview="<tool_call><name>WebSearch</name>..."
```
(No "Parsed tool calls" message)

**Cause:** The response contains a tool call but in a format our parser doesn't recognize. The parser supports JSON and XML `<function_calls>` format — other XML tag names (e.g., `<tool_call>`) are not supported.

**Solutions:**
- Check the full content at TRACE level to see the exact format
- Look for the format detection log: `Parsed tool calls from JSON format` or `Parsed tool calls from XML format` — if neither appears, the format is unrecognized
- File an issue with the exact format Copilot used
- We may need to add parsers for additional formats (see `src/tools/parser.rs`)

### Issue: No tool injection happening
**Symptom:** No "Injecting tools" message in logs

**Causes:**
- Request doesn't contain any tools
- Claude Code not configured to send tools

**Solutions:**
```bash
# Start with debug logging
copilot-adapter start --log-level debug

# Check Claude Code configuration
echo $ANTHROPIC_BASE_URL  # Should be http://127.0.0.1:6767
```

## Sharing Debug Info

If you need to report an issue, please include:

1. **Log excerpt** showing:
   - The "Received request" line (model, num_messages)
   - The "Injecting tools" line (if any)
   - The "Raw response" line (content_preview, finish_reason)
   - The "Parsed tool calls" line (if any)

2. **Full content** (TRACE level) of one problematic response

3. **Context:**
   - What question did you ask?
   - What tool(s) did you expect to be called?
   - What actually happened?

## Log File Locations

- **With `--log-file`:** Specified path
- **Without `--log-file`:** Console only (stdout/stderr)
- **With debug scripts:** `debug_responses_YYYYMMDD_HHMMSS.log`

## Next Steps

If tool calls aren't working after reviewing the logs:

1. Share your findings in a GitHub issue
2. See TOOLS-SUPPORT.design.md for background on how the system works
3. See TOOLS-SUPPORT.plan.md Epic 9 for planned reliability improvements
