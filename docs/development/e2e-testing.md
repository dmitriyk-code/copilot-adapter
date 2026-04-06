# Manual End-to-End Testing Procedures

This document describes manual E2E test procedures for the Copilot Adapter.
These tests require a GitHub account with an active Copilot subscription and
cannot be fully automated.

> **Prerequisites:**
> - A GitHub account with an active GitHub Copilot subscription
> - The `copilot-adapter` binary built and available in your `PATH`
> - `curl` installed for API testing

---

## Test 1: Authentication Flow

**Purpose:** Verify the GitHub OAuth device flow completes successfully.

### Steps

1. **If previously authenticated, clear credentials:**
   ```bash
   copilot-adapter logout
   ```

2. **Initiate authentication:**
   ```bash
   copilot-adapter auth
   ```

3. **Expected output:**
   ```
     To authenticate, visit:

       https://github.com/login/device

     And enter this code: XXXX-XXXX

     Press Enter to open in browser (or wait to continue manually)...
     Waiting for authorization...
   ```

4. **Complete the flow:**
   - Open the verification URL in your browser
   - Enter the displayed code
   - Authorize the application on GitHub

5. **Expected result:**
   ```
     ✓ Authentication successful! Copilot token obtained.
     Credentials stored securely.
   ```

### Verification

```bash
# Running auth again should report already authenticated
copilot-adapter auth
# Expected: "Already authenticated. Use --force to re-authenticate."
```

### Failure Scenarios to Test

- **Deny authorization:** Click "Cancel" in the browser → should show error message
- **Let code expire:** Wait without entering the code (~15 min) → should show timeout error
- **Force re-auth:** `copilot-adapter auth --force` → should start a new flow even when already authenticated

---

## Test 2: Server Start and Health Check

**Purpose:** Verify the adapter starts and responds to health checks.

### Steps

1. **Start in foreground:**
   ```bash
   copilot-adapter start
   ```

2. **In another terminal, check health:**
   ```bash
   curl http://127.0.0.1:6767/health
   ```

3. **Expected response:**
   ```json
   {"status": "ok"}
   ```

4. **Stop with Ctrl+C.**

### Custom Port

```bash
copilot-adapter start --port 9090
curl http://127.0.0.1:9090/health
```

---

## Test 3: Daemon Lifecycle

**Purpose:** Verify background daemon start, status, and stop.

### Steps

1. **Start as daemon:**
   ```bash
   copilot-adapter start --daemon
   ```
   Expected output (varies by platform):
   - **Unix:** Returns to prompt silently (process daemonized)
   - **Windows:** `Adapter started in background (PID XXXXX)`

2. **Check status:**
   ```bash
   copilot-adapter status
   ```
   Expected: `Adapter running on PID XXXXX, port 6767`

3. **Verify server is accessible:**
   ```bash
   curl http://127.0.0.1:6767/health
   ```

4. **Stop the daemon:**
   ```bash
   copilot-adapter stop
   ```
   Expected: `Adapter stopped (was PID XXXXX).`

5. **Verify stopped:**
   ```bash
   copilot-adapter status
   ```
   Expected: `Adapter is not running.`

### Failure Scenarios

- **Double start:** Try starting when already running → should print error message and exit
- **Stop when not running:** `copilot-adapter stop` → should print error about not running

---

## Test 4: Models Endpoint

**Purpose:** Verify the `/v1/models` endpoints return valid model listings, including dynamic fetching, caching, and fallback behaviour.

### Steps

1. **Start the adapter** (ensure auth is complete).

2. **List all models:**
   ```bash
   curl -s http://127.0.0.1:6767/v1/models | python3 -m json.tool
   ```

3. **Expected response format:**
   ```json
   {
     "object": "list",
     "data": [
       {
         "id": "gpt-4",
         "object": "model",
         "created": 1686935002,
         "owned_by": "github-copilot"
       },
       ...
     ]
   }
   ```

4. **Get specific model:**
   ```bash
   curl -s http://127.0.0.1:6767/v1/models/gpt-4 | python3 -m json.tool
   ```

5. **Unknown model (expect 404):**
   ```bash
   curl -s -w "\nHTTP Status: %{http_code}\n" http://127.0.0.1:6767/v1/models/nonexistent
   ```

### Test 4a: Fresh Start Fetches from API

**Purpose:** Verify that a fresh adapter start fetches models from the Copilot API (not the static list).

1. **Start with debug logging:**
   ```bash
   copilot-adapter start --log-level debug
   ```

2. **Request models:**
   ```bash
   curl -s http://127.0.0.1:6767/v1/models | python3 -m json.tool
   ```

3. **Check logs for:**
   ```
   Models cache miss, fetching from Copilot API
   Fetched models from Copilot API
   ```

4. **Verify** the response contains models from the Copilot API (e.g., models beyond the static fallback list like `claude-sonnet-4` or `o1-mini`).

### Test 4b: Second Request Uses Cache

**Purpose:** Verify that a second request within the TTL returns cached data without an API call.

1. **Start with debug logging (if not already):**
   ```bash
   copilot-adapter start --log-level debug
   ```

2. **Request models twice:**
   ```bash
   curl -s http://127.0.0.1:6767/v1/models > /dev/null
   curl -s http://127.0.0.1:6767/v1/models > /dev/null
   ```

3. **Check logs:**
   - First request: `Models cache miss, fetching from Copilot API`
   - Second request: `Models cache hit`

4. **Verify** only one API call was made (only one `Fetched models from Copilot API` log line).

### Test 4c: Request After TTL Refetches

**Purpose:** Verify that after the cache TTL expires, the next request fetches fresh data.

1. **Start with a short TTL:**
   ```bash
   copilot-adapter start --log-level debug --models-cache-ttl 10
   ```

2. **Request models, wait for TTL, then request again:**
   ```bash
   curl -s http://127.0.0.1:6767/v1/models > /dev/null   # Fetch & cache
   sleep 11                                                 # Wait for TTL expiry
   curl -s http://127.0.0.1:6767/v1/models > /dev/null   # Should refetch
   ```

3. **Check logs for two separate `fetching from Copilot API` entries.**

### Test 4d: Network Disconnect Triggers Fallback

**Purpose:** Verify that when the Copilot API is unreachable, the adapter returns the static fallback list.

1. **Option A — Disconnect network:**
   ```bash
   copilot-adapter start --log-level debug
   # Disconnect from the internet (disable Wi-Fi/Ethernet)
   curl -s http://127.0.0.1:6767/v1/models | python3 -m json.tool
   # Reconnect
   ```

2. **Option B — Use an invalid token:**
   ```bash
   copilot-adapter logout
   copilot-adapter start --log-level debug
   curl -s http://127.0.0.1:6767/v1/models | python3 -m json.tool
   ```

3. **Expected:**
   - Response status: 200 (never fails)
   - Response body contains fallback models: `gpt-4o`, `gpt-4`, `gpt-4-turbo`, `gpt-3.5-turbo`
   - **Option A:** Logs contain a warning: `Failed to fetch models from Copilot API, using fallback list`
   - **Option B:** Logs contain a warning: `Failed to obtain token for models fetch, using fallback list`

### Test 4e: Static Models Mode

**Purpose:** Verify `--static-models` flag bypasses API calls entirely.

1. **Start with static models:**
   ```bash
   copilot-adapter start --log-level debug --static-models
   ```

2. **Request models:**
   ```bash
   curl -s http://127.0.0.1:6767/v1/models | python3 -m json.tool
   ```

3. **Expected:**
   - Response contains exactly 4 models: `gpt-4o`, `gpt-4`, `gpt-4-turbo`, `gpt-3.5-turbo`
   - Logs show: `Static models mode enabled, returning fallback list`
   - No `fetching from Copilot API` log entries

---

## Test 5: Non-Streaming Messages

**Purpose:** Verify messages work in non-streaming mode.

### Steps

1. **Send a simple request:**
   ```bash
   curl -s -X POST http://127.0.0.1:6767/v1/messages \
     -H "Content-Type: application/json" \
     -H "x-api-key: dummy" \
     -H "anthropic-version: 2023-06-01" \
     -d '{
       "model": "claude-3-5-sonnet-20241022",
       "max_tokens": 1024,
       "messages": [{"role": "user", "content": "Say hello in one sentence."}]
     }' | python3 -m json.tool
   ```

2. **Expected response format:**
   ```json
   {
     "id": "msg_...",
     "type": "message",
     "role": "assistant",
     "content": [
       {
         "type": "text",
         "text": "Hello! How can I assist you today?"
       }
     ],
     "model": "claude-3-5-sonnet-20241022",
     "stop_reason": "end_turn"
   }
   ```

3. **Verify:**
   - Response has valid JSON structure
   - `type` is `"message"`
   - `content` array has at least one entry
   - `content[0].type` is `"text"`
   - `content[0].text` is non-empty

### With System Message

```bash
curl -s -X POST http://127.0.0.1:6767/v1/messages \
  -H "Content-Type: application/json" \
  -H "x-api-key: dummy" \
  -H "anthropic-version: 2023-06-01" \
  -d '{
    "model": "claude-3-5-sonnet-20241022",
    "max_tokens": 1024,
    "system": "You are a helpful assistant. Respond in exactly 3 words.",
    "messages": [
      {"role": "user", "content": "What is Rust?"}
    ]
  }' | python3 -m json.tool
```

---

## Test 6: Streaming Messages

**Purpose:** Verify SSE streaming returns proper Server-Sent Events.

### Steps

1. **Send a streaming request:**
   ```bash
   curl -N -X POST http://127.0.0.1:6767/v1/messages \
     -H "Content-Type: application/json" \
     -H "x-api-key: dummy" \
     -H "anthropic-version: 2023-06-01" \
     -d '{
       "model": "claude-3-5-sonnet-20241022",
       "max_tokens": 1024,
       "messages": [{"role": "user", "content": "Count from 1 to 5."}],
       "stream": true
     }'
   ```

2. **Expected output format:**
   ```
   event: message_start
   data: {"type":"message_start","message":{"id":"msg_...","type":"message","role":"assistant",...}}

   event: content_block_start
   data: {"type":"content_block_start","index":0,"content_block":{"type":"text","text":""}}

   event: content_block_delta
   data: {"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"1"}}

   ...

   event: message_stop
   data: {"type":"message_stop"}
   ```

3. **Verify:**
   - Each event has `event:` and `data:` lines
   - Stream starts with `message_start`
   - Content arrives as `content_block_delta` events
   - Stream ends with `message_stop`

---

## Test 7: Concurrent Clients

**Purpose:** Verify the adapter handles multiple simultaneous requests.

### Steps

1. **Start the adapter.**

2. **Open 5 terminal windows and run simultaneously:**

   Terminal 1:
   ```bash
   curl -N -X POST http://127.0.0.1:6767/v1/messages \
     -H "Content-Type: application/json" \
     -H "x-api-key: dummy" \
     -H "anthropic-version: 2023-06-01" \
     -d '{"model": "claude-3-5-sonnet-20241022", "max_tokens": 1024, "messages": [{"role": "user", "content": "Terminal 1"}], "stream": true}'
   ```

   Terminal 2–5: Same command with different content.

3. **Verify:**
   - All 5 requests receive complete responses
   - No timeouts or connection errors
   - Streaming responses arrive concurrently (not sequentially)

### Automated Concurrent Test

```bash
# Launch 10 requests in parallel using background jobs
for i in $(seq 1 10); do
  curl -s -X POST http://127.0.0.1:6767/v1/messages \
    -H "Content-Type: application/json" \
    -H "x-api-key: dummy" \
    -H "anthropic-version: 2023-06-01" \
    -d "{\"model\": \"claude-3-5-sonnet-20241022\", \"max_tokens\": 1024, \"messages\": [{\"role\": \"user\", \"content\": \"Request $i\"}]}" \
    -o "/tmp/copilot-test-$i.json" &
done
wait

# Check all responses
for i in $(seq 1 10); do
  echo "Request $i: $(python3 -c "import json; d=json.load(open('/tmp/copilot-test-$i.json')); print(d.get('type', 'ERROR'))")"
done
```

---

## Test 8: Error Handling

**Purpose:** Verify proper error responses for invalid inputs.

### Empty Messages

```bash
curl -s -w "\nHTTP Status: %{http_code}\n" -X POST http://127.0.0.1:6767/v1/messages \
  -H "Content-Type: application/json" \
  -H "x-api-key: dummy" \
  -H "anthropic-version: 2023-06-01" \
  -d '{"model": "claude-3-5-sonnet-20241022", "max_tokens": 1024, "messages": []}'
```
Expected: HTTP 400, error format with `type: "invalid_request_error"`.

### Invalid JSON

```bash
curl -s -w "\nHTTP Status: %{http_code}\n" -X POST http://127.0.0.1:6767/v1/messages \
  -H "Content-Type: application/json" \
  -H "x-api-key: dummy" \
  -H "anthropic-version: 2023-06-01" \
  -d 'not valid json'
```
Expected: HTTP 400 or 422.

### Unauthenticated (After Logout)

```bash
copilot-adapter logout
copilot-adapter start
# In another terminal:
curl -s -w "\nHTTP Status: %{http_code}\n" -X POST http://127.0.0.1:6767/v1/messages \
  -H "Content-Type: application/json" \
  -H "x-api-key: dummy" \
  -H "anthropic-version: 2023-06-01" \
  -d '{"model": "claude-3-5-sonnet-20241022", "max_tokens": 1024, "messages": [{"role": "user", "content": "Hello"}]}'
```
Expected: HTTP 401, `type: "authentication_error"`.

---

## Test 9: Logging

**Purpose:** Verify logging configuration works correctly.

### Steps

1. **Start with debug logging to file:**
   ```bash
   copilot-adapter start --log-level debug --log-file /tmp/adapter.log
   ```

2. **Send a request** (in another terminal).

3. **Check logs:**
   ```bash
   cat /tmp/adapter.log
   ```

4. **Expected log entries:**
   - `Request received` with method, path, request_id
   - `Request completed` with status, duration
   - Token refresh messages (if applicable)
   - `Sending request to Copilot API`

### Environment Variable

```bash
RUST_LOG=trace copilot-adapter start
```

---

## Test 10: Claude Code Integration

**Purpose:** Verify end-to-end integration with Claude Code.

### Steps

1. **Start the adapter:**
   ```bash
   copilot-adapter start --daemon
   ```

2. **Configure environment:**
   ```bash
   export ANTHROPIC_BASE_URL=http://127.0.0.1:6767
   export ANTHROPIC_API_KEY=dummy
   ```

3. **Start Claude Code** and send a message.

4. **Verify:**
   - Claude Code connects to the adapter
   - Responses are received correctly
   - Streaming works (tokens appear incrementally)

5. **Clean up:**
   ```bash
   copilot-adapter stop
   ```

---

## Test 11: Tool Call (Non-Streaming, Anthropic Format)

**Purpose:** Verify tool/function calling works via `/v1/messages`.

> **Prerequisites:**
> - Adapter started
> - Authenticated with GitHub

### Steps

1. **Start the adapter:**
   ```bash
   copilot-adapter start
   ```

2. **Send a request with tool definitions:**
   ```bash
   curl -s -X POST http://127.0.0.1:6767/v1/messages \
     -H "Content-Type: application/json" \
     -H "x-api-key: dummy" \
     -H "anthropic-version: 2023-06-01" \
     -d '{
       "model": "claude-3-5-sonnet-20241022",
       "max_tokens": 1024,
       "messages": [{"role": "user", "content": "What directory am I in?"}],
       "tools": [{
         "name": "bash",
         "description": "Run a bash command",
         "input_schema": {
           "type": "object",
           "properties": {
             "command": {"type": "string", "description": "The command to run"}
           },
           "required": ["command"]
         }
       }]
     }' | python3 -m json.tool
   ```

3. **Expected response:**
   - `content` array should contain a `tool_use` block with `name`, `id`, and `input`
   - `stop_reason` should be `"tool_use"`
   - Text blocks should not contain raw XML tool call tags

### Verification

- Response has valid `tool_use` content blocks
- `input` contains valid JSON arguments
- Content does not contain `<function_calls>` or `<invoke>` XML blocks (these are stripped during parsing)

---

## Test 12: Tool Call (Streaming, Anthropic Format)

**Purpose:** Verify tool calls are detected in streaming responses.

### Steps

1. **Send a streaming request with tools:**
   ```bash
   curl -N -X POST http://127.0.0.1:6767/v1/messages \
     -H "Content-Type: application/json" \
     -H "x-api-key: dummy" \
     -H "anthropic-version: 2023-06-01" \
     -d '{
       "model": "claude-3-5-sonnet-20241022",
       "max_tokens": 1024,
       "messages": [{"role": "user", "content": "List files in the current directory"}],
       "stream": true,
       "tools": [{
         "name": "bash",
         "description": "Run a bash command",
         "input_schema": {
           "type": "object",
           "properties": {
             "command": {"type": "string"}
           },
           "required": ["command"]
         }
       }]
     }'
   ```

2. **Expected output:**
   - SSE events with content block deltas
   - A `content_block_start` event with `type: "tool_use"` containing the parsed tool call
   - The tool use block should have `name` and `id` fields
   - Stream ends with `message_stop`

---

## Test 13: Multi-Turn Conversation with Tool Results

**Purpose:** Verify the adapter handles tool result messages in follow-up requests.

### Steps

1. **Send a request with a tool result from a previous turn:**
   ```bash
   curl -s -X POST http://127.0.0.1:6767/v1/messages \
     -H "Content-Type: application/json" \
     -H "x-api-key: dummy" \
     -H "anthropic-version: 2023-06-01" \
     -d '{
       "model": "claude-3-5-sonnet-20241022",
       "max_tokens": 1024,
       "messages": [
         {"role": "user", "content": "What directory am I in?"},
         {"role": "assistant", "content": [
           {"type": "text", "text": "Let me check."},
           {"type": "tool_use", "id": "toolu_123", "name": "bash", "input": {"command": "pwd"}}
         ]},
         {"role": "user", "content": [
           {"type": "tool_result", "tool_use_id": "toolu_123", "content": "/home/user/project"}
         ]}
       ],
       "tools": [{
         "name": "bash",
         "description": "Run a bash command",
         "input_schema": {"type": "object", "properties": {"command": {"type": "string"}}, "required": ["command"]}
       }]
     }' | python3 -m json.tool
   ```

2. **Expected response:**
   - The model should receive the tool result and generate a follow-up response
   - Response should be valid Anthropic-format JSON
   - `content` array should have at least one `text` block

---

## Test 14: Tool Call with Multiple Tools

**Purpose:** Verify tool support with multiple tool definitions via the `/v1/messages` endpoint.

### Steps

1. **Start the adapter** (if not already running).

2. **Send an Anthropic-format request with multiple tools:**
   ```bash
   curl -s -X POST http://127.0.0.1:6767/v1/messages \
     -H "Content-Type: application/json" \
     -H "x-api-key: dummy" \
     -H "anthropic-version: 2023-06-01" \
     -d '{
       "model": "claude-3-5-sonnet-20241022",
       "max_tokens": 1024,
       "messages": [{"role": "user", "content": "What directory am I in?"}],
       "tools": [
         {
           "name": "bash",
           "description": "Run a bash command",
           "input_schema": {
             "type": "object",
             "properties": {
               "command": {"type": "string"}
             },
             "required": ["command"]
           }
         },
         {
           "name": "read_file",
           "description": "Read a file from disk",
           "input_schema": {
             "type": "object",
             "properties": {
               "path": {"type": "string"}
             },
             "required": ["path"]
           }
         }
       ]
     }' | python3 -m json.tool
   ```

3. **Expected response:**
   - `content` array should contain a `tool_use` block with `name`, `id`, and `input`
   - `stop_reason` should be `"tool_use"`
   - Text blocks should not contain fenced JSON

---

## Test 14b: XML Tool Call Format Verification

**Purpose:** Verify that the adapter uses XML format for tool injection and correctly parses XML tool call responses.

### Steps

1. **Start the adapter with trace logging:**
   ```bash
   copilot-adapter start --log-level trace
   ```

2. **Send a tool request:**
   ```bash
   curl -s -X POST http://127.0.0.1:6767/v1/messages \
     -H "Content-Type: application/json" \
     -H "x-api-key: dummy" \
     -H "anthropic-version: 2023-06-01" \
     -d '{
       "model": "claude-3-5-sonnet-20241022",
       "max_tokens": 1024,
       "messages": [{"role": "user", "content": "What directory am I in?"}],
       "tools": [{
         "name": "bash",
         "description": "Run a bash command",
         "input_schema": {
           "type": "object",
           "properties": {
             "command": {"type": "string", "description": "The command to run"}
           },
           "required": ["command"]
         }
       }]
     }' | python3 -m json.tool
   ```

3. **Check trace logs for XML tool injection:**
   - The outgoing request to Copilot API should contain XML tool definitions in the system prompt:
     ```xml
     <tool_description>
     <tool_name>bash</tool_name>
     <description>Run a bash command</description>
     <parameters>
     <parameter>
     <name>command</name>
     <type>string</type>
     <description>The command to run</description>
     <required>true</required>
     </parameter>
     </parameters>
     </tool_description>
     ```
   - The tool usage instructions should reference `<function_calls>` and `<invoke>` XML tags
   - There should be **no** JSON-format tool injection (`{"function_call": ...}`)

4. **Verify response parsing:**
   - The response should contain `tool_use` content blocks (Anthropic format)
   - Any `<function_calls>` XML blocks from the model response should be stripped from text content
   - The `input` field should contain the parsed parameters as JSON

### Verification

- Trace logs confirm XML format is used for injection
- Response `tool_use` blocks have correctly parsed parameters
- No JSON tool format artifacts in logs or responses

---

## Test 15: Claude Code with Tools Integration

**Purpose:** Verify Claude Code's native tool use works through the adapter.

### Steps

1. **Start the adapter:**
   ```bash
   copilot-adapter start --daemon
   ```

2. **Configure Claude Code:**
   ```bash
   export ANTHROPIC_BASE_URL=http://127.0.0.1:6767
   export ANTHROPIC_API_KEY=dummy
   ```

3. **Run Claude Code and test tool use:**
   ```bash
   claude
   ```
   Type: `What is my current working directory?`

4. **Expected behavior:**
   - Claude Code should execute the `bash` tool to run `pwd` (or equivalent)
   - The actual directory path should be returned
   - Claude should not just describe the command but actually execute it

5. **Clean up:**
   ```bash
   copilot-adapter stop
   ```

---

## Test 16: Image Upload (Anthropic Format — Base64)

**Purpose:** Verify that base64 image uploads via `/v1/messages` are translated to OpenAI `image_url` format and forwarded successfully.

> **Prerequisites:**
> - Adapter is running and authenticated
> - Use a vision-capable model (e.g., `gpt-4o`)

### Steps

1. **Start the adapter with debug logging:**
   ```bash
   copilot-adapter start --log-level debug
   ```

2. **Send an image upload request (base64):**
   ```bash
   curl -s -X POST http://127.0.0.1:6767/v1/messages \
     -H "Content-Type: application/json" \
     -H "x-api-key: dummy" \
     -H "anthropic-version: 2023-06-01" \
     -d '{
       "model": "gpt-4o",
       "max_tokens": 1024,
       "messages": [{
         "role": "user",
         "content": [
           {"type": "text", "text": "Describe this image in one sentence."},
           {
             "type": "image",
             "source": {
               "type": "base64",
               "media_type": "image/png",
               "data": "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNk+M9QDwADhgGAWjR9awAAAABJRU5ErkJggg=="
             }
           }
         ]
       }]
     }' | python3 -m json.tool
   ```

   > **Tip:** The base64 data above is a 1×1 red pixel PNG. For a more meaningful test, replace it with a real image encoded via `base64 -w0 photo.jpg` (Linux) or `base64 -i photo.jpg` (macOS).

3. **Expected response:**
   ```json
   {
     "id": "msg_...",
     "type": "message",
     "role": "assistant",
     "content": [
       {
         "type": "text",
         "text": "The image shows a small red dot..."
       }
     ],
     "model": "gpt-4o",
     "stop_reason": "end_turn"
   }
   ```

4. **Verify:**
   - Response HTTP status is 200 (not 422)
   - Response has valid Anthropic message format
   - `content[0].type` is `"text"`
   - `content[0].text` is non-empty and describes the image
   - No deserialization errors in adapter logs

### Log Verification

Check the adapter logs for:
- **No** `Failed to deserialize` errors
- The request should be translated to OpenAI multimodal format with `image_url` content blocks
- If debug logging is enabled, you should see the translated request being sent to the Copilot API

---

## Test 17: Image Upload (Anthropic Format — URL)

**Purpose:** Verify that URL-based image references are passed through correctly.

### Steps

1. **Send an image upload request (URL):**
   ```bash
   curl -s -X POST http://127.0.0.1:6767/v1/messages \
     -H "Content-Type: application/json" \
     -H "x-api-key: dummy" \
     -H "anthropic-version: 2023-06-01" \
     -d '{
       "model": "gpt-4o",
       "max_tokens": 1024,
       "messages": [{
         "role": "user",
         "content": [
           {"type": "text", "text": "What do you see in this image?"},
           {
             "type": "image",
             "source": {
               "type": "url",
               "url": "https://upload.wikimedia.org/wikipedia/commons/thumb/4/47/PNG_transparency_demonstration_1.png/280px-PNG_transparency_demonstration_1.png"
             }
           }
         ]
       }]
     }' | python3 -m json.tool
   ```

2. **Verify:**
   - Response HTTP status is 200
   - Model describes the image content
   - No errors in adapter logs

---

## Test 18: Mixed Content (Text + Image + Document)

**Purpose:** Verify that mixed content messages are handled correctly — images translated, documents skipped with warning.

### Steps

1. **Send a mixed content request:**
   ```bash
   curl -s -X POST http://127.0.0.1:6767/v1/messages \
     -H "Content-Type: application/json" \
     -H "x-api-key: dummy" \
     -H "anthropic-version: 2023-06-01" \
     -d '{
       "model": "gpt-4o",
       "max_tokens": 1024,
       "messages": [{
         "role": "user",
         "content": [
           {"type": "text", "text": "Analyze the following:"},
           {
             "type": "image",
             "source": {
               "type": "base64",
               "media_type": "image/png",
               "data": "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNk+M9QDwADhgGAWjR9awAAAABJRU5ErkJggg=="
             }
           },
           {
             "type": "document",
             "source": {
               "type": "base64",
               "media_type": "application/pdf",
               "data": "JVBERi0xLjQKMSAwIG9iago8PAovVHlwZSAvQ2F0YWxvZwo+PgplbmRvYmoKdHJhaWxlcgo8PAovUm9vdCAxIDAgUgo+Pg=="
             },
             "title": "test-document.pdf"
           }
         ]
       }]
     }' | python3 -m json.tool
   ```

2. **Verify:**
   - Response HTTP status is 200 (not 422)
   - The model processes the text and image (document is silently skipped)
   - Adapter logs contain a warning: `Document content blocks are not supported by OpenAI format; skipping`
   - The warning includes the document title `test-document.pdf`

---

## Test 19: Image Upload with Cache Control

**Purpose:** Verify that `cache_control` metadata on content blocks is accepted without errors.

### Steps

1. **Send a request with cache_control:**
   ```bash
   curl -s -X POST http://127.0.0.1:6767/v1/messages \
     -H "Content-Type: application/json" \
     -H "x-api-key: dummy" \
     -H "anthropic-version: 2023-06-01" \
     -d '{
       "model": "gpt-4o",
       "max_tokens": 1024,
       "messages": [{
         "role": "user",
         "content": [
           {
             "type": "text",
             "text": "Describe this image.",
             "cache_control": {"type": "ephemeral"}
           },
           {
             "type": "image",
             "source": {
               "type": "base64",
               "media_type": "image/png",
               "data": "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNk+M9QDwADhgGAWjR9awAAAABJRU5ErkJggg=="
             },
             "cache_control": {"type": "ephemeral"}
           }
         ]
       }]
     }' | python3 -m json.tool
   ```

2. **Verify:**
   - Response HTTP status is 200 (not 422)
   - `cache_control` is accepted without errors
   - Response is a valid Anthropic message

---

## Test 20: Claude Code Image Upload (Integration)

**Purpose:** Verify that uploading an image through Claude Code works end-to-end.

### Steps

1. **Start the adapter:**
   ```bash
   copilot-adapter start --daemon --log-level debug --log-file /tmp/copilot-adapter-image-test.log
   ```

2. **Configure Claude Code:**
   ```bash
   export ANTHROPIC_BASE_URL=http://127.0.0.1:6767
   export ANTHROPIC_API_KEY=dummy
   ```

3. **Run Claude Code and upload an image:**
   ```bash
   claude
   ```
   Use Claude Code's image upload feature (drag-and-drop or paste) and ask:
   `What is in this image?`

---

## Test 21: Native Tools — Basic Streaming

**Purpose:** Verify that `--native-tools` mode passes tool definitions natively to the Copilot API and streams responses progressively.

### Steps

1. **Start the adapter in native tools mode:**
   ```bash
   copilot-adapter start --native-tools --log-level debug
   ```

2. **Send a streaming request with a tool:**
   ```bash
   curl -N -X POST http://127.0.0.1:6767/v1/messages \
     -H "Content-Type: application/json" \
     -H "x-api-key: dummy" \
     -H "anthropic-version: 2023-06-01" \
     -d '{
       "model": "claude-3-5-sonnet-20241022",
       "max_tokens": 1024,
       "messages": [{"role": "user", "content": "What directory am I in?"}],
       "stream": true,
       "tools": [{
         "name": "bash",
         "description": "Run a bash command",
         "input_schema": {
           "type": "object",
           "properties": {
             "command": {"type": "string", "description": "The command to run"}
           },
           "required": ["command"]
         }
       }]
     }'
   ```

3. **Expected output:**
   - SSE events stream **progressively** (tokens appear incrementally, not all at once)
   - `content_block_start` event with `type: "tool_use"` for the tool call
   - `content_block_delta` events with `input_json_delta` for tool arguments
   - Stream ends with `message_stop`

4. **Log verification:**
   - Debug logs should show native tools being sent to the Copilot API (no XML injection)
   - No `Injecting tools into prompt` log message (that's XML mode only)

### Comparison with XML Mode

Start the adapter without `--native-tools` and run the same request. Compare:
- **XML mode:** Response arrives all at once (buffered)
- **Native mode:** Response streams progressively

---

## Test 22: Native Tools — MCP Tools with Typed Parameters

**Purpose:** Verify that MCP tools with typed parameters (number, boolean) work correctly in native tools mode without validation errors.

### Steps

1. **Start the adapter in native tools mode:**
   ```bash
   copilot-adapter start --native-tools --log-level debug
   ```

2. **Send a request with typed parameters:**
   ```bash
   curl -s -X POST http://127.0.0.1:6767/v1/messages \
     -H "Content-Type: application/json" \
     -H "x-api-key: dummy" \
     -H "anthropic-version: 2023-06-01" \
     -d '{
       "model": "claude-3-5-sonnet-20241022",
       "max_tokens": 1024,
       "messages": [{"role": "user", "content": "Search for Rust tutorials, limit to 5 results"}],
       "tools": [{
         "name": "mcp__search__search_web",
         "description": "Search the web",
         "input_schema": {
           "type": "object",
           "properties": {
             "query": {"type": "string", "description": "Search query"},
             "limit": {"type": "number", "description": "Maximum results"},
             "safe_search": {"type": "boolean", "description": "Enable safe search"}
           },
           "required": ["query"]
         }
       }]
     }' | python3 -m json.tool
   ```

3. **Expected response:**
   - `content` contains a `tool_use` block
   - `input.limit` is a number (e.g., `5`), not a string (`"5"`)
   - `input.safe_search` is a boolean (e.g., `true`), not a string (`"true"`)
   - No MCP validation errors

---

## Test 23: Native Tools — Tool Name Truncation

**Purpose:** Verify that long MCP tool names (>64 characters) are automatically truncated with a hash suffix and restored in responses.

### Steps

1. **Start the adapter in native tools mode:**
   ```bash
   copilot-adapter start --native-tools --log-level debug
   ```

2. **Send a request with a long tool name:**
   ```bash
   curl -s -X POST http://127.0.0.1:6767/v1/messages \
     -H "Content-Type: application/json" \
     -H "x-api-key: dummy" \
     -H "anthropic-version: 2023-06-01" \
     -d '{
       "model": "claude-3-5-sonnet-20241022",
       "max_tokens": 1024,
       "messages": [{"role": "user", "content": "Search for code"}],
       "tools": [{
         "name": "mcp__codemogger__codemogger_search_with_extra_long_suffix_name",
         "description": "Search code in the repository",
         "input_schema": {
           "type": "object",
           "properties": {
             "query": {"type": "string"}
           },
           "required": ["query"]
         }
       }]
     }' | python3 -m json.tool
   ```

3. **Expected behavior:**
   - The adapter truncates the name to ≤64 characters with a hash suffix
   - Debug logs show the truncation: original name → truncated name
   - If the model calls the tool, the response `tool_use` block has the **original** full name restored

---

## Test 24: XML Fallback with `--xml-tools`

**Purpose:** Verify that `--xml-tools` forces XML-based tool injection and that the adapter correctly parses XML tool call responses.

### Steps

1. **Start the adapter in XML mode:**
   ```bash
   copilot-adapter start --xml-tools --log-level trace
   ```

2. **Send a tool request:**
   ```bash
   curl -s -X POST http://127.0.0.1:6767/v1/messages \
     -H "Content-Type: application/json" \
     -H "x-api-key: dummy" \
     -H "anthropic-version: 2023-06-01" \
     -d '{
       "model": "claude-3-5-sonnet-20241022",
       "max_tokens": 1024,
       "messages": [{"role": "user", "content": "What directory am I in?"}],
       "tools": [{
         "name": "bash",
         "description": "Run a bash command",
         "input_schema": {
           "type": "object",
           "properties": {
             "command": {"type": "string", "description": "The command to run"}
           },
           "required": ["command"]
         }
       }]
     }' | python3 -m json.tool
   ```

3. **Log verification:**
   - Trace logs show XML tool definitions injected into the system prompt
   - The `Injecting tools into prompt` log message appears
   - No native tools sent in the request body

4. **Expected response:**
   - `content` contains `tool_use` blocks (parsed from XML)
   - `stop_reason` is `"tool_use"`

---

## Test 25: XML Fallback — Parameter Type Coercion

**Purpose:** Verify that the XML fallback path correctly coerces parameter types using schema information from the request.

### Steps

1. **Start the adapter in XML mode:**
   ```bash
   copilot-adapter start --xml-tools --log-level debug
   ```

2. **Send a request with typed parameters:**
   ```bash
   curl -s -X POST http://127.0.0.1:6767/v1/messages \
     -H "Content-Type: application/json" \
     -H "x-api-key: dummy" \
     -H "anthropic-version: 2023-06-01" \
     -d '{
       "model": "claude-3-5-sonnet-20241022",
       "max_tokens": 1024,
       "messages": [{"role": "user", "content": "Search for Rust tutorials, limit to 5 results"}],
       "tools": [{
         "name": "mcp__search__search_web",
         "description": "Search the web",
         "input_schema": {
           "type": "object",
           "properties": {
             "query": {"type": "string", "description": "Search query"},
             "limit": {"type": "number", "description": "Maximum results"},
             "safe_search": {"type": "boolean", "description": "Enable safe search"}
           },
           "required": ["query"]
         }
       }]
     }' | python3 -m json.tool
   ```

3. **Expected response:**
   - `content` contains a `tool_use` block
   - `input.limit` is a number (coerced from XML string), not a string
   - `input.safe_search` is a boolean (coerced from XML string), not a string
   - The `ToolRegistry` performs schema-aware coercion to prevent MCP validation errors

---

## Test 26: Native Tools — Mutual Exclusivity of Flags

**Purpose:** Verify that `--native-tools` and `--xml-tools` cannot be used simultaneously and that the adapter rejects invalid flag combinations.

### Steps

1. **Attempt to start with both flags:**
   ```bash
   copilot-adapter start --native-tools --xml-tools
   ```

2. **Expected result:**
   - The adapter exits with a non-zero exit code
   - An error message indicates the flags are mutually exclusive (e.g., "the argument '--native-tools' cannot be used with '--xml-tools'")
   - The adapter does **not** start listening on any port

3. **Verify `--native-tools` alone works:**
   ```bash
   copilot-adapter start --native-tools --log-level debug
   ```
   - The adapter starts successfully
   - Debug logs indicate native tools mode is active

4. **Verify `--xml-tools` alone works:**
   ```bash
   copilot-adapter start --xml-tools --log-level debug
   ```
   - The adapter starts successfully
   - Debug logs indicate XML tools mode is active

5. **Verify default (no flag) works:**
   ```bash
   copilot-adapter start --log-level debug
   ```
   - The adapter starts successfully
   - Default mode is XML injection (no native tools log message)

---

## Test 27: Native Tools — Claude Code Integration

**Purpose:** Verify that native tools mode works end-to-end with Claude Code, including progressive streaming of tool calls.

### Steps

1. **Start the adapter in native tools mode:**
   ```bash
   copilot-adapter start --native-tools --log-level debug
   ```

2. **Configure Claude Code:**
   ```bash
   export ANTHROPIC_BASE_URL=http://127.0.0.1:6767
   export ANTHROPIC_API_KEY=dummy
   ```

3. **Run Claude Code and use a tool:**
   ```bash
   claude
   ```
   Ask Claude Code to perform an operation that invokes a tool, e.g.:
   `What files are in the current directory?`

4. **Verify progressive streaming:**
   - Text and tool calls stream **progressively** (tokens appear incrementally)
   - Tool execution completes successfully (e.g., file listing is displayed)
   - No errors or warnings in the Claude Code UI

5. **Clean up:**
   ```bash
   copilot-adapter stop
   ```

---

## Test 28: Root Path Handler

**Purpose:** Verify `GET /` and `HEAD /` return 200 OK (eliminates 404 log noise from Claude Code health probes).

### Steps

1. **Start the adapter:**
   ```bash
   copilot-adapter start --log-level debug
   ```

2. **Test GET / returns JSON body:**
   ```bash
   curl -v http://127.0.0.1:6767/
   ```

3. **Expected response:**
   ```
   < HTTP/1.1 200 OK
   < content-type: application/json
   {"status":"ok"}
   ```

4. **Test HEAD / returns 200 with empty body:**
   ```bash
   curl -I http://127.0.0.1:6767/
   ```

5. **Expected response:**
   ```
   HTTP/1.1 200 OK
   content-type: application/json
   ```
   (No body content for HEAD request.)

6. **Test unsupported methods return 405:**
   ```bash
   curl -X POST http://127.0.0.1:6767/
   curl -X PUT http://127.0.0.1:6767/
   ```

7. **Expected:** `405 Method Not Allowed` for both.

### Verification

- Check adapter logs show `status=200` for root path requests (not `status=404`).
- No authentication header is required.
- Response time should be <1ms (NFR1).

---

## Test 29: Token Counting Endpoint

**Purpose:** Verify `POST /v1/messages/count_tokens` returns accurate token counts for various request formats.

### Steps

1. **Start the adapter** (if not already running).

2. **Simple text message:**
   ```bash
   curl -s -X POST http://127.0.0.1:6767/v1/messages/count_tokens \
     -H "Content-Type: application/json" \
     -d '{
       "model": "claude-sonnet-4-20250514",
       "messages": [{"role": "user", "content": "Hello!"}]
     }'
   ```

3. **Expected response:**
   ```json
   {"input_tokens": N}
   ```
   where `N` is approximately 4–8 (a simple greeting plus per-message overhead). The exact value depends on the tokenizer.

4. **With system prompt:**
   ```bash
   curl -s -X POST http://127.0.0.1:6767/v1/messages/count_tokens \
     -H "Content-Type: application/json" \
     -d '{
       "model": "claude-sonnet-4-20250514",
       "messages": [{"role": "user", "content": "Hello!"}],
       "system": "You are a helpful assistant."
     }'
   ```

5. **Expected:** Token count should be higher than simple message (system prompt tokens added). Approximately 10-20 tokens.

6. **With tool definitions:**
   ```bash
   curl -s -X POST http://127.0.0.1:6767/v1/messages/count_tokens \
     -H "Content-Type: application/json" \
     -d '{
       "model": "claude-sonnet-4-20250514",
       "messages": [{"role": "user", "content": "Search for foo"}],
       "tools": [{
         "name": "Grep",
         "description": "Search files",
         "input_schema": {
           "type": "object",
           "properties": {"pattern": {"type": "string"}}
         }
       }]
     }'
   ```

7. **Expected:** Token count should include tool definition overhead. Approximately 40-80 tokens.

### Error Cases

8. **Missing messages field (invalid JSON structure):**
   ```bash
   curl -s -w "\nHTTP Status: %{http_code}\n" \
     -X POST http://127.0.0.1:6767/v1/messages/count_tokens \
     -H "Content-Type: application/json" \
     -d '{"model": "claude-sonnet-4-20250514"}'
   ```

9. **Expected:** `400 Bad Request` or `422 Unprocessable Entity`.

10. **Missing model field:**
    ```bash
    curl -s -w "\nHTTP Status: %{http_code}\n" \
      -X POST http://127.0.0.1:6767/v1/messages/count_tokens \
      -H "Content-Type: application/json" \
      -d '{"messages": [{"role": "user", "content": "test"}]}'
    ```

11. **Expected:** `400 Bad Request` or `422 Unprocessable Entity` (model is a required field per FR8).

12. **Invalid JSON:**
    ```bash
    curl -s -w "\nHTTP Status: %{http_code}\n" \
      -X POST http://127.0.0.1:6767/v1/messages/count_tokens \
      -H "Content-Type: application/json" \
      -d 'not valid json'
    ```

13. **Expected:** `400 Bad Request` or `422 Unprocessable Entity`.

### Verification

- Token counts are consistent across repeated calls for the same input.
- Counts increase proportionally with message length.
- No authentication header is required (similar to `/health` — this is a utility endpoint that does not proxy to the upstream API).

---

## Test 30: Token Counting — Performance

**Purpose:** Verify token counting meets the <10ms response time target (NFR2).

### Steps

1. **Start the adapter** (if not already running).

2. **Warm up the tokenizer** (first request initializes the BPE encoder):
   ```bash
   curl -s -X POST http://127.0.0.1:6767/v1/messages/count_tokens \
     -H "Content-Type: application/json" \
     -d '{"model": "claude-sonnet-4-20250514", "messages": [{"role": "user", "content": "warmup"}]}'
   ```

3. **Time a typical request:**
   ```bash
   # Linux/macOS
   time curl -s -X POST http://127.0.0.1:6767/v1/messages/count_tokens \
     -H "Content-Type: application/json" \
     -d '{"model": "claude-sonnet-4-20250514", "messages": [{"role": "user", "content": "Hello, how are you doing today?"}]}'

   # Windows (PowerShell)
   Measure-Command {
     Invoke-WebRequest -Uri http://127.0.0.1:6767/v1/messages/count_tokens `
       -Method POST -ContentType "application/json" `
       -Body '{"model": "claude-sonnet-4-20250514", "messages": [{"role": "user", "content": "Hello, how are you doing today?"}]}'
   } | Select-Object TotalMilliseconds
   ```

4. **Expected:** Total response time <10ms (excluding network overhead for localhost, the actual computation should be well under 10ms).

5. **Test with larger payload (~10KB text):**
   ```bash
   # Linux/macOS
   LARGE_TEXT=$(python3 -c "print('x' * 10000)")
   time curl -s -X POST http://127.0.0.1:6767/v1/messages/count_tokens \
     -H "Content-Type: application/json" \
     -d "{\"model\": \"claude-sonnet-4-20250514\", \"messages\": [{\"role\": \"user\", \"content\": \"$LARGE_TEXT\"}]}"

   # Windows (PowerShell)
   $largeText = "x" * 10000
   $body = @{model="claude-sonnet-4-20250514"; messages=@(@{role="user"; content=$largeText})} | ConvertTo-Json -Depth 3
   Measure-Command {
     Invoke-WebRequest -Uri http://127.0.0.1:6767/v1/messages/count_tokens `
       -Method POST -ContentType "application/json" -Body $body
   } | Select-Object TotalMilliseconds
   ```

6. **Expected:** Should still complete in <50ms even for large payloads.

### Verification

- Typical requests (1-50 tokens): <10ms
- Large requests (~10,000 characters): <50ms
- Results are deterministic (same input → same count)

---

## Test 31: Token Counting — Claude Code Integration

**Purpose:** Verify that Claude Code can use the token counting endpoint when configured with the adapter.

### Steps

1. **Start the adapter:**
   ```bash
   copilot-adapter start --log-level debug
   ```

2. **Configure Claude Code:**
   ```bash
   export ANTHROPIC_BASE_URL=http://127.0.0.1:6767
   export ANTHROPIC_API_KEY=dummy
   ```

3. **Run Claude Code:**
   ```bash
   claude
   ```

4. **Send a message and observe:**
   - Claude Code may call `/v1/messages/count_tokens` for context window management
   - Check adapter logs for `POST /v1/messages/count_tokens` requests
   - If visible, verify the response is `200 OK` with `{"input_tokens": N}`

5. **Note:** Claude Code may not always call this endpoint — it depends on the version and internal logic. The key verification is that the endpoint responds correctly when called.

6. **Clean up:**
   ```bash
   copilot-adapter stop
   ```

---

## Test 32: Daemon Auth E2E

**Purpose:** Verify that `start --daemon` without existing credentials triggers the interactive device flow (instead of exiting with an error), then starts the daemon normally.

> **Prerequisites:**
> - The adapter binary is built and in `PATH`
> - A GitHub account with an active Copilot subscription
> - No credentials stored (or cleared via `logout`)

### Steps

1. **Clear any existing credentials:**
   ```bash
   copilot-adapter logout
   ```

2. **Start the adapter in daemon mode (no credentials):**
   ```bash
   copilot-adapter start --daemon
   ```

3. **Expected output:**
   - The adapter detects missing credentials and prints:
     ```
     No authentication credentials found.
     Starting authentication flow...
     ```
   - The device flow begins — you see:
     ```
       To authenticate, visit:

         https://github.com/login/device

       And enter this code: XXXX-XXXX

       Press Enter to open in browser (or wait to continue manually)...
       Waiting for authorization...
     ```
   - After the code display, the adapter offers to open the browser (10-second timeout).
     Pressing Enter opens the URL; waiting proceeds without the browser.
   - **The adapter does NOT exit with an error.** This validates Epic 1's fix.

4. **Complete the device flow:**
   - Open the URL in your browser
   - Enter the device code
   - Authorize the application

5. **Expected:** Auth succeeds, and the daemon starts:
   - **Unix:** Returns to prompt silently (daemonized)
   - **Windows:** `Adapter started in background (PID XXXXX)` followed by guidance output

6. **Verify the daemon is running:**
   ```bash
   copilot-adapter status
   ```
   Expected output:
   ```
   Adapter running on PID XXXXX
     Port:       6767
     Version:    X.Y.Z
     Started at: 2026-04-02T...
   ```

7. **Verify the server responds:**
   ```bash
   curl http://127.0.0.1:6767/health
   ```
   Expected: `{"status":"ok"}`

8. **Stop the daemon:**
   ```bash
   copilot-adapter stop
   ```
   Expected: `Adapter stopped (was PID XXXXX).`

9. **Verify stopped:**
   ```bash
   copilot-adapter status
   ```
   Expected: `Adapter is not running.`

### Re-auth Scenario

Also verify that `start --daemon` with an **expired/invalid** stored token triggers re-auth:

1. Manually corrupt the credentials file (or wait for the token to expire).
2. Run `copilot-adapter start --daemon`.
3. Expected: Prints `Stored token is invalid or expired: ...` then `Starting re-authentication...` and begins the device flow.

### Failure Scenarios

- **Cancel in browser:** Deny the authorization → should show error, daemon does NOT start
- **Timeout:** Let the device code expire (~15 min) → should show timeout error

---

## Test 33: Home Directory Storage E2E

**Purpose:** Verify that credentials and status files are stored under `~/.copilot-adapter/profiles/default/` with correct structure and content.

> **Prerequisites:**
> - The adapter binary is built and in `PATH`
> - A GitHub account with an active Copilot subscription
> - Run `copilot-adapter logout` first to start clean

### Steps

1. **Authenticate:**
   ```bash
   copilot-adapter auth
   ```
   Complete the device flow in your browser.

2. **Verify credentials file exists:**

   ```bash
   # Linux/macOS
   ls -la ~/.copilot-adapter/profiles/default/github-copilot.json

   # Windows (PowerShell)
   Get-Item "$env:USERPROFILE\.copilot-adapter\profiles\default\github-copilot.json"
   ```

   Expected: The file exists and is non-empty. It is human-readable JSON in version 2 format, e.g.:
   ```json
   {
     "version": 2,
     "storage": "dpapi",
     "github_token": "<base64-encoded-encrypted-blob>"
   }
   ```
   On macOS/Linux with a keyring available, `"storage"` will be `"keyring"` and `"github_token"` will be absent (the token is in the OS keyring).

3. **Start the adapter:**
   ```bash
   copilot-adapter start --daemon
   ```

4. **Verify status file exists with correct fields:**

   ```bash
   # Linux/macOS
   cat ~/.copilot-adapter/profiles/default/status.json

   # Windows (PowerShell)
   Get-Content "$env:USERPROFILE\.copilot-adapter\profiles\default\status.json"
   ```

   Expected JSON structure:
   ```json
   {
     "pid": 12345,
     "port": 6767,
     "started_at": "2026-04-02T01:30:45.123456789+00:00",
     "version": "0.1.0"
   }
   ```

   Verify:
   - `pid` is a valid process ID (matches a running process)
   - `port` is `6767` (default)
   - `started_at` is a valid ISO 8601 / RFC 3339 timestamp
   - `version` matches the adapter's cargo package version

5. **Verify rich status output:**
   ```bash
   copilot-adapter status
   ```
   Expected output (with all four fields):
   ```
   Adapter running on PID 12345
     Port:       6767
     Version:    0.1.0
     Started at: 2026-04-02T01:30:45.123456789+00:00
   ```

6. **Stop the adapter:**
   ```bash
   copilot-adapter stop
   ```
   Expected: `Adapter stopped (was PID 12345).`

7. **Verify status file is removed:**
   ```bash
   # Linux/macOS
   ls ~/.copilot-adapter/profiles/default/status.json
   # Expected: No such file or directory

   # Windows (PowerShell)
   Test-Path "$env:USERPROFILE\.copilot-adapter\profiles\default\status.json"
   # Expected: False
   ```

### Directory Structure Verification

After running `auth` and `start`, the directory tree should look like:

```
~/.copilot-adapter/
└── profiles/
    └── default/
        ├── github-copilot.json   # Native-encrypted credentials (DPAPI on Windows; keyring sentinel on macOS/Linux)
        └── status.json           # Runtime status (removed after stop)
```

```bash
# Linux/macOS
find ~/.copilot-adapter -type f | sort

# Windows (PowerShell)
Get-ChildItem -Recurse "$env:USERPROFILE\.copilot-adapter" -File | Select-Object FullName
```

---

## Test 34: Multi-Instance Profiles E2E

**Purpose:** Verify that multiple named profiles can be created, authenticated, started on different ports simultaneously, managed with `--all`, and cleaned up with `profiles delete`.

> **Prerequisites:**
> - The adapter binary is built and in `PATH`
> - A GitHub account with an active Copilot subscription
> - Default profile is authenticated (`copilot-adapter auth`)
> - Port 8080 is not in use by another application

### Steps

1. **Create a "work" profile:**
   ```bash
   copilot-adapter profiles create work
   ```
   Expected: `Profile 'work' created.`

2. **Verify the profile directory was created:**
   ```bash
   # Linux/macOS
   ls -d ~/.copilot-adapter/profiles/work/

   # Windows (PowerShell)
   Test-Path "$env:USERPROFILE\.copilot-adapter\profiles\work"
   # Expected: True
   ```

3. **List profiles:**
   ```bash
   copilot-adapter profiles list
   ```
   Expected output:
   ```
   Profiles:
     default (stopped)
     work (stopped)
   ```

4. **Authenticate the "work" profile:**
   ```bash
   copilot-adapter auth -P work
   ```
   Complete the device flow. Verify that credentials are stored in the work profile's directory:
   ```bash
   # Linux/macOS
   ls ~/.copilot-adapter/profiles/work/github-copilot.json

   # Windows (PowerShell)
   Test-Path "$env:USERPROFILE\.copilot-adapter\profiles\work\github-copilot.json"
   # Expected: True
   ```

5. **Start the default profile:**
   ```bash
   copilot-adapter start --daemon
   ```

6. **Start the "work" profile on port 8080:**
   ```bash
   copilot-adapter start -P work -p 8080 --daemon
   ```

7. **Verify both instances are running:**
   ```bash
   curl http://127.0.0.1:6767/health
   # Expected: {"status":"ok"}

   curl http://127.0.0.1:8080/health
   # Expected: {"status":"ok"}
   ```

8. **Check status of all profiles:**
   ```bash
   copilot-adapter status --all
   ```
   Expected output (both profiles shown):
   ```
   Profile 'default': running
     PID:        XXXXX
     Port:       6767
     Version:    X.Y.Z
     Started at: 2026-04-02T...
   Profile 'work': running
     PID:        YYYYY
     Port:       8080
     Version:    X.Y.Z
     Started at: 2026-04-02T...
   ```

9. **Check single profile status:**
   ```bash
   copilot-adapter status -P work
   ```
   Expected:
   ```
   Adapter running on PID YYYYY
     Port:       8080
     Version:    X.Y.Z
     Started at: 2026-04-02T...
     Profile:    work
   ```

10. **Stop all profiles:**
    ```bash
    copilot-adapter stop --all
    ```
    Expected (output order may vary — `read_dir` traversal is filesystem-dependent):
    ```
    Stopped profile 'default' (was PID XXXXX).
    Stopped profile 'work' (was PID YYYYY).
    ```

11. **Verify all stopped:**
    ```bash
    copilot-adapter status --all
    ```
    Expected: `No running profiles.`

12. **Delete the "work" profile:**
    ```bash
    copilot-adapter profiles delete work
    ```
    Expected: `Profile 'work' deleted.`

13. **Verify the profile directory is removed:**
    ```bash
    # Linux/macOS
    ls -d ~/.copilot-adapter/profiles/work/
    # Expected: No such file or directory

    # Windows (PowerShell)
    Test-Path "$env:USERPROFILE\.copilot-adapter\profiles\work"
    # Expected: False
    ```

14. **Verify default profile remains:**
    ```bash
    copilot-adapter profiles list
    ```
    Expected:
    ```
    Profiles:
      default (stopped)
    ```

### Port Conflict Detection

15. **Start default profile:**
    ```bash
    copilot-adapter start --daemon
    ```

16. **Try to start "work" on the same port (should fail):**
    ```bash
    copilot-adapter profiles create work
    copilot-adapter auth -P work
    copilot-adapter start -P work -p 6767 --daemon
    ```
    Expected error: `Error: Port 6767 is already in use by profile 'default'`

17. **Clean up:**
    ```bash
    copilot-adapter stop
    copilot-adapter profiles delete work
    ```

### Failure Scenarios

- **Delete running profile:** `copilot-adapter profiles delete work` while it's running → Expected: `Profile 'work' is currently running (PID XXXXX). Stop it first.`
- **Delete default:** `copilot-adapter profiles delete default` → Expected: `Error: Cannot delete the default profile`
- **Create duplicate:** `copilot-adapter profiles create work` twice → Expected: `Error: Profile 'work' already exists`
- **Start nonexistent profile:** `copilot-adapter start -P nonexistent` → Expected: `Error: Profile 'nonexistent' does not exist`

---

## Test 35: Windows DPAPI Credential Storage

**Purpose:** Verify that on Windows the adapter stores credentials using DPAPI encryption in the `github-copilot.json` file.

> **Prerequisites:**
> - Running on Windows
> - The adapter binary is built and in `PATH`
> - A GitHub account with an active Copilot subscription

### Steps

1. **Clean state:**
   ```powershell
   copilot-adapter logout
   Remove-Item -Recurse -Force "$env:USERPROFILE\.copilot-adapter" -ErrorAction SilentlyContinue
   ```

2. **Authenticate:**
   ```powershell
   copilot-adapter auth
   ```
   Follow the device flow in the browser.

3. **Verify credential file format:**
   ```powershell
   Get-Content "$env:USERPROFILE\.copilot-adapter\profiles\default\github-copilot.json" | python -m json.tool
   ```

4. **Expected output:**
   ```json
   {
     "version": 2,
     "storage": "dpapi",
     "github_token": "<base64-encoded DPAPI-encrypted blob>"
   }
   ```

5. **Verify adapter works with DPAPI credentials:**
   ```powershell
   copilot-adapter start
   # In another terminal:
   curl -s -X POST http://127.0.0.1:6767/v1/messages `
     -H "Content-Type: application/json" `
     -H "x-api-key: dummy" `
     -H "anthropic-version: 2023-06-01" `
     -d '{"model":"claude-3-5-sonnet-20241022","max_tokens":50,"messages":[{"role":"user","content":"Say hello"}]}'
   ```

6. **Expected:** Valid response with assistant message. Token decrypted via DPAPI and used to obtain a Copilot API token.

### Verification Checklist

- [ ] File exists at `~/.copilot-adapter/profiles/default/github-copilot.json`
- [ ] File contains `"version": 2`
- [ ] File contains `"storage": "dpapi"`
- [ ] File contains `"github_token"` with a base64-encoded string
- [ ] Adapter starts successfully with the stored credentials
- [ ] API requests return valid responses

---

## Test 36: macOS/Linux Keyring Credential Storage

**Purpose:** Verify that on macOS/Linux the adapter stores credentials using the OS keyring with a sentinel JSON file.

> **Prerequisites:**
> - Running on macOS or Linux
> - On Linux: a Secret Service provider must be running (GNOME Keyring, KDE Wallet, or `pass`)
> - A GitHub account with an active Copilot subscription

### Steps

1. **Clean state:**
   ```bash
   copilot-adapter logout
   rm -rf ~/.copilot-adapter
   ```

2. **Authenticate:**
   ```bash
   copilot-adapter auth
   ```
   Follow the device flow in the browser.

3. **Verify credential file format:**
   ```bash
   cat ~/.copilot-adapter/profiles/default/github-copilot.json | python3 -m json.tool
   ```

4. **Expected output:**
   ```json
   {
     "version": 2,
     "storage": "keyring"
   }
   ```
   Note: `github_token` field is **absent** — the token is stored in the OS keyring.

5. **Verify adapter works with keyring credentials:**
   ```bash
   copilot-adapter start
   curl -s http://127.0.0.1:6767/health
   ```

6. **Expected:** `{"status": "ok"}` — adapter decrypts token from keyring successfully.

### Verification Checklist

- [ ] Sentinel file exists at `~/.copilot-adapter/profiles/default/github-copilot.json`
- [ ] File contains `"version": 2`
- [ ] File contains `"storage": "keyring"`
- [ ] File does **not** contain a `"github_token"` field
- [ ] Adapter starts and serves requests using the keyring-stored token

---

## Test 37: Automatic Migration from XOR Format

**Purpose:** Verify that the adapter automatically migrates credentials from the old XOR-obfuscated `credentials.json` to the new `github-copilot.json` format.

> **Prerequisites:**
> - An old `credentials.json` file from a previous adapter version, or a manually created one
> - The new adapter binary

### Steps

1. **Setup — Create old-format credentials:**

   If you have a previous adapter version installed:
   ```bash
   # Use old version to authenticate (creates credentials.json)
   old-copilot-adapter auth
   ```

   Or manually create a test file (advanced — requires matching the XOR key derivation):
   ```bash
   # Ensure no new-format file exists
   rm -f ~/.copilot-adapter/profiles/default/github-copilot.json
   # Place old-format file in the profile directory
   cp /path/to/old/credentials.json ~/.copilot-adapter/profiles/default/credentials.json
   ```

2. **Start the new adapter with debug logging:**
   ```bash
   copilot-adapter start --log-level debug
   ```

3. **Check logs for migration messages:**
   ```
   Migrating credentials from XOR format to native encryption
   Successfully migrated credentials to new format
   Deleted old XOR credentials file
   ```

4. **Verify file state:**
   ```bash
   # Old file should be gone
   ls ~/.copilot-adapter/profiles/default/credentials.json
   # Expected: No such file or directory

   # New file should exist
   cat ~/.copilot-adapter/profiles/default/github-copilot.json | python3 -m json.tool
   # Expected: {"version": 2, "storage": "dpapi" or "keyring", ...}
   ```

5. **Verify adapter works with migrated credentials:**
   ```bash
   curl -s http://127.0.0.1:6767/health
   # Expected: {"status": "ok"}
   ```

### Verification Checklist

- [ ] Old `credentials.json` is deleted after migration
- [ ] New `github-copilot.json` is created with version 2 format
- [ ] Logs show migration info messages
- [ ] Adapter functions correctly with migrated token

---

## Test 38: Edge Case — Both Credential Files Exist

**Purpose:** Verify that when both `credentials.json` (old) and `github-copilot.json` (new) exist, the adapter removes the old file and uses the new one.

### Steps

1. **Setup — Create both files:**
   ```bash
   PROFILE_DIR=~/.copilot-adapter/profiles/default

   # Ensure new-format credentials exist (authenticate first if needed)
   copilot-adapter auth

   # Manually place an old-format file alongside it
   echo "fake-old-data" > "$PROFILE_DIR/credentials.json"
   ```

2. **Start the adapter:**
   ```bash
   copilot-adapter start --log-level debug
   ```

3. **Verify:**
   ```bash
   # Old file should be removed
   ls "$PROFILE_DIR/credentials.json"
   # Expected: No such file or directory

   # New file should be untouched
   cat "$PROFILE_DIR/github-copilot.json" | python3 -m json.tool
   # Expected: valid JSON with version 2
   ```

4. **Check logs:**
   ```
   Removed old XOR credentials file (new format already exists)
   ```

### Verification Checklist

- [ ] Old `credentials.json` is removed
- [ ] New `github-copilot.json` is preserved and used
- [ ] Adapter logs a message about removing the old file
- [ ] Adapter works correctly

---

## Test 39: Corrupted XOR Credential File

**Purpose:** Verify that a corrupted old `credentials.json` is handled gracefully — deleted with a warning, and the user is prompted to re-authenticate.

### Steps

1. **Setup — Create a corrupted old file:**
   ```bash
   PROFILE_DIR=~/.copilot-adapter/profiles/default
   mkdir -p "$PROFILE_DIR"

   # Remove any new-format file
   rm -f "$PROFILE_DIR/github-copilot.json"

   # Write corrupted data
   echo "this is not valid XOR-encrypted JSON" > "$PROFILE_DIR/credentials.json"
   ```

2. **Start the adapter with debug logging:**
   ```bash
   copilot-adapter start --log-level debug
   ```

3. **Check logs for warnings:**
   ```
   Failed to read old XOR credentials. Please run `copilot-adapter auth` to re-authenticate.
   Deleted old XOR credentials file
   ```

4. **Verify:**
   ```bash
   # Corrupted file should be deleted
   ls "$PROFILE_DIR/credentials.json"
   # Expected: No such file or directory

   # No new file should have been created (migration failed)
   ls "$PROFILE_DIR/github-copilot.json"
   # Expected: No such file or directory

   # Adapter should prompt for authentication
   copilot-adapter status
   # May report not running or require re-auth
   ```

5. **Re-authenticate:**
   ```bash
   copilot-adapter auth
   # Should start fresh device flow
   ```

### Verification Checklist

- [ ] Corrupted `credentials.json` is deleted
- [ ] Warning logged with `re-authenticate` guidance
- [ ] No corrupted data propagated to new file
- [ ] Fresh `copilot-adapter auth` works after cleanup

---

## Test Summary Checklist

| # | Test Name | Category |
|---|-----------|----------|
| 1 | Authentication Flow | Auth |
| 2 | Server Start and Health Check | Operations |
| 3 | Daemon Lifecycle | Operations |
| 4 | Models Endpoint | Models |
| 5 | Non-Streaming Messages | API |
| 6 | Streaming Messages | API |
| 7 | Concurrent Clients | API |
| 8 | Error Handling | API |
| 9 | Logging | Operations |
| 10 | Claude Code Integration | Integration |
| 11 | Tool Call (Non-Streaming, Anthropic Format) | Tools |
| 12 | Tool Call (Streaming, Anthropic Format) | Tools |
| 13 | Multi-Turn Conversation with Tool Results | Tools |
| 14 | Tool Call with Multiple Tools | Tools |
| 14b | XML Tool Call Format Verification | Tools |
| 15 | Claude Code with Tools Integration | Integration |
| 16 | Image Upload (Anthropic Format — Base64) | Vision |
| 17 | Image Upload (Anthropic Format — URL) | Vision |
| 18 | Mixed Content (Text + Image + Document) | Vision |
| 19 | Image Upload with Cache Control | Vision |
| 20 | Claude Code Image Upload (Integration) | Vision |
| 21 | Native Tools — Basic Streaming | Native Tools |
| 22 | Native Tools — MCP Tools with Typed Parameters | Native Tools |
| 23 | Native Tools — Tool Name Truncation | Native Tools |
| 24 | XML Fallback with `--xml-tools` | Native Tools |
| 25 | XML Fallback — Parameter Type Coercion | Native Tools |
| 26 | Native Tools — Mutual Exclusivity of Flags | Native Tools |
| 27 | Native Tools — Claude Code Integration | Native Tools |
| 28 | Root Path Handler | Health / Compatibility |
| 29 | Token Counting Endpoint | API |
| 30 | Token Counting — Performance | API |
| 31 | Token Counting — Claude Code Integration | Integration |
| 32 | Daemon Auth E2E | Profiles / Auth |
| 33 | Home Directory Storage E2E | Profiles / Storage |
| 34 | Multi-Instance Profiles E2E | Profiles / Multi-Instance |
| 35 | Windows DPAPI Credential Storage | Credential Storage |
| 36 | macOS/Linux Keyring Credential Storage | Credential Storage |
| 37 | Automatic Migration from XOR Format | Credential Migration |
| 38 | Edge Case — Both Credential Files Exist | Credential Migration |
| 39 | Corrupted XOR Credential File | Credential Migration |
| 40 | Prompt-Too-Long Recovery | Context Window |
| 41 | Truncated Tool Call Escalation | Context Window |
| 42 | 1M Context Model Activation | Context Window |
| 43 | Effort Level Forwarding | Effort / Thinking |
| 44 | Thinking Blocks in Conversation History | Effort / Thinking |

---

## Test 40: Prompt-Too-Long Recovery

**Purpose:** Verify that when the Copilot API returns a prompt-too-long error, the adapter translates it into Anthropic format so Claude Code triggers context compaction.

> **Prerequisites:**
> - The adapter is built and authenticated
> - A long-running Claude Code session that approaches the token limit

### Steps

1. **Start copilot-adapter with debug logging:**
   ```bash
   copilot-adapter start --log-level debug
   ```

2. **Start a Claude Code session** with a long conversation.

3. **Continue until the prompt approaches 168K tokens** (keep sending messages,
   pasting large files, etc.).

4. **Observe that Claude Code receives "prompt too long" error.**

5. **Verify Claude Code triggers context compaction** (visible in Claude Code output — it will automatically summarize and trim conversation history).

6. **Verify the adapter logs show:**
   ```
   Translating prompt-too-long error to Anthropic format
   ```

### Expected Result

- Claude Code compacts context and continues the session without crashing.
- The adapter translates the Copilot 400 `model_max_prompt_tokens_exceeded` error to
  Anthropic's `prompt_too_long` format with the message:
  `"prompt is too long: N tokens > M maximum"`.

### Verification Checklist

- [ ] Claude Code compacts context automatically
- [ ] Adapter log shows "Translating prompt-too-long error to Anthropic format"
- [ ] Session continues working after compaction

---

## Test 41: Truncated Tool Call Escalation

**Purpose:** Verify that when a tool call is truncated due to output token limit, the adapter emits a text notice that triggers Claude Code's max_tokens escalation logic.

> **Prerequisites:**
> - The adapter is built and authenticated

### Steps

1. **Start copilot-adapter with debug logging:**
   ```bash
   copilot-adapter start --log-level debug
   ```

2. **Start a Claude Code session.**

3. **Ask Claude to write a very large file** (>8K tokens of content), e.g.:
   ```
   Write a 500-line Python module with comprehensive docstrings for a REST API client
   ```

4. **Observe that the first attempt uses the default max_tokens** (8K).

5. **Observe that the tool call is truncated** — the adapter log should show:
   ```
   Dropping truncated tool_use block
   ```
   And Claude Code receives a text block with:
   `[Tool call to "Write" was truncated due to output token limit]`

6. **Observe that Claude Code escalates max_tokens and retries** — the second
   request should have a higher max_tokens value (e.g., 64K).

7. **Verify the second attempt with the escalated token budget succeeds.**

### Expected Result

- File write succeeds on retry with escalated token budget.
- No tool_use block is emitted for the truncated call.
- Claude Code automatically retries with higher max_tokens.

### Verification Checklist

- [ ] First attempt truncated (adapter log: "Dropping truncated tool_use block")
- [ ] Claude Code receives truncation notice text block
- [ ] Claude Code escalates max_tokens and retries
- [ ] Second attempt succeeds

---

## Test 42: 1M Context Model Activation

**Purpose:** Verify that the `anthropic-beta: context-1m-*` header triggers the adapter to append `-1m` to the model name sent to Copilot API.

> **Prerequisites:**
> - The adapter is built and authenticated
> - A Copilot subscription with access to 1M context models

### Steps

1. **Start copilot-adapter with debug logging:**
   ```bash
   copilot-adapter start --log-level debug
   ```

2. **Start Claude Code and select "Opus (1M context)"** from the model picker.

3. **Send a message** and observe the adapter logs.

4. **Verify the adapter log shows:**
   ```
   1M context beta detected, selecting Copilot 1M model variant
   ```

5. **Verify the adapter log shows the outgoing request model:**
   ```
   model="claude-opus-4.6-1m"
   ```

6. **Verify the conversation works normally** with the 1M model.

7. **Optionally:** Start a very long conversation and verify it doesn't hit the
   standard 168K limit (the 1M model should support up to ~1M tokens).

### Expected Result

- Adapter forwards requests to `claude-opus-4.6-1m`.
- Longer conversations are supported without hitting the standard token limit.

### Verification Checklist

- [ ] Adapter log shows "1M context beta detected"
- [ ] Outgoing request uses model name with `-1m` suffix
- [ ] Conversation works normally with 1M model
- [ ] No double-appending of `-1m` suffix if model already contains it

---

## Test 43: Effort Level Forwarding

**Purpose:** Verify that Claude Code's `/effort` command is translated to the OpenAI `reasoning.effort` parameter in requests to the Copilot API.

> **Prerequisites:**
> - The adapter is built and authenticated

### Steps

1. **Start copilot-adapter with trace logging:**
   ```bash
   copilot-adapter start --log-level trace
   ```

2. **Start Claude Code** and run:
   ```
   /effort high
   ```

3. **Send a message** and observe the adapter trace logs.

4. **Verify the adapter log shows:**
   ```
   Translating effort level
   ```
   With fields: `anthropic_effort="high"` and `openai_effort="high"`.

5. **Verify the outgoing request to Copilot API contains:**
   ```json
   "reasoning": {"effort": "high"}
   ```

6. **Verify the conversation works normally.**

7. **Run `/effort low`** and send another message.

8. **Verify the outgoing request contains:**
   ```json
   "reasoning": {"effort": "low"}
   ```

### Expected Result

- Effort level is forwarded to Copilot API in the `reasoning` object.
- The `"max"` Anthropic effort level is translated to `"high"` (OpenAI's highest level).
- Conversations work normally with all effort levels.

### Verification Checklist

- [ ] `/effort high` → `reasoning.effort = "high"` in outgoing request
- [ ] `/effort low` → `reasoning.effort = "low"` in outgoing request
- [ ] `/effort max` → `reasoning.effort = "high"` (downgraded)
- [ ] Conversation works normally at all effort levels

---

## Test 44: Thinking Blocks in Conversation History

**Purpose:** Verify that thinking blocks (from Claude's extended thinking feature) in conversation history are accepted by the adapter and stripped before forwarding to the Copilot API.

> **Prerequisites:**
> - The adapter is built and authenticated
> - A model that supports thinking (e.g., Claude Sonnet 4)

### Steps

1. **Start copilot-adapter with trace logging:**
   ```bash
   copilot-adapter start --log-level trace
   ```

2. **Start a Claude Code session** (thinking should be enabled by default for
   supported models).

3. **Have a multi-turn conversation** (at least 3 turns).

4. **Observe that subsequent requests include thinking blocks** in conversation
   history — visible in the trace logs as incoming request bodies containing
   `"type": "thinking"` content blocks in assistant messages.

5. **Verify the adapter does NOT fail** with deserialization errors.

6. **Verify the outgoing requests to Copilot API do NOT contain** thinking or
   redacted_thinking content blocks — they should be stripped.

7. **Verify the conversation continues normally.**

### Expected Result

- Thinking blocks are accepted and stripped from the conversation history.
- No deserialization errors or panics.
- Outgoing requests contain only text and tool_use content (no thinking blocks).
- Temperature is suppressed when thinking is active.

### Verification Checklist

- [ ] Thinking blocks accepted in incoming requests (no errors)
- [ ] Thinking blocks stripped from outgoing requests
- [ ] RedactedThinking blocks also stripped
- [ ] Temperature suppressed when thinking is present
- [ ] Conversation continues normally

---

## Test 45: Proactive Token Refresh

**Purpose:** Verify that the background auto-refresh task proactively refreshes the
Copilot token before it expires, even when no requests are being processed.

### Steps

1. **Start the adapter with debug logging:**
   ```bash
   copilot-adapter start --log-level debug
   ```

2. **Verify the auto-refresh task started** — look for this log entry at startup:
   ```
   Token auto-refresh task started
   ```

3. **Make one request** to trigger the initial token acquisition:
   ```bash
   curl -s -X POST http://127.0.0.1:6767/v1/messages \
     -H "Content-Type: application/json" \
     -H "x-api-key: dummy" \
     -H "anthropic-version: 2023-06-01" \
     -d '{"model": "claude-sonnet-4-20250514", "max_tokens": 64, "messages": [{"role": "user", "content": "Say hi"}]}'
   ```

4. **Wait approximately 25 minutes** without making any further requests.
   (Copilot tokens expire after ~30 minutes; the auto-refresh fires 5 minutes before.)

5. **Check the logs for a proactive refresh:**
   ```
   Copilot token auto-refreshed successfully
   ```
   This should appear ~25 minutes after the initial request, before the token expires.

6. **After the auto-refresh fires, make another request:**
   ```bash
   curl -s -X POST http://127.0.0.1:6767/v1/messages \
     -H "Content-Type: application/json" \
     -H "x-api-key: dummy" \
     -H "anthropic-version: 2023-06-01" \
     -d '{"model": "claude-sonnet-4-20250514", "max_tokens": 64, "messages": [{"role": "user", "content": "Still here?"}]}'
   ```
   This request should succeed immediately (no 401 retry).

### Expected Result

- The auto-refresh log appears ~5 minutes before the token's 30-minute expiry.
- No 401 errors occur during or after the idle period.
- The second request succeeds without triggering a lazy token refresh.

### Verification Checklist

- [ ] `"Token auto-refresh task started"` appears in startup logs
- [ ] `"Copilot token auto-refreshed successfully"` appears ~25 minutes after first request
- [ ] No 401 errors in the logs during the idle period
- [ ] Subsequent request succeeds without a visible token refresh

---

## Test 46: System Prompt Block Separation

**Purpose:** Verify that multi-block system prompts are separated by `"\n\n"` in the
outgoing OpenAI request, not concatenated without separators.

### Steps

1. **Start the adapter with trace logging:**
   ```bash
   copilot-adapter start --log-level trace
   ```

2. **Make any request through Claude Code** (or send a request with a multi-block
   system prompt):
   ```bash
   curl -s -X POST http://127.0.0.1:6767/v1/messages \
     -H "Content-Type: application/json" \
     -H "x-api-key: dummy" \
     -H "anthropic-version: 2023-06-01" \
     -d '{
       "model": "claude-sonnet-4-20250514",
       "max_tokens": 64,
       "system": [
         {"type": "text", "text": "Block A: billing header cch=00000;"},
         {"type": "text", "text": "Block B: You are Claude Code."}
       ],
       "messages": [{"role": "user", "content": "Hello"}]
     }'
   ```

3. **In the trace log, find the `OUTGOING` direction entry** for the request to the
   Copilot API. Look for the system message content.

4. **Verify the system blocks are separated by `\n\n`:**
   ```
   "content": "Block A: billing header cch=00000;\n\nBlock B: You are Claude Code."
   ```
   NOT:
   ```
   "content": "Block A: billing header cch=00000;Block B: You are Claude Code."
   ```

### Expected Result

- The outgoing system message shows `"\n\n"` between blocks.
- No run-on concatenation of adjacent blocks.

### Verification Checklist

- [ ] Trace log shows `OUTGOING` request to Copilot API
- [ ] System message content has `"\n\n"` between blocks
- [ ] No run-on text between adjacent system blocks
