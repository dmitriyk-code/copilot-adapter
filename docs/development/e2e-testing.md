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
     To authenticate, open the following URL in your browser:

       https://github.com/login/device

     And enter this code: XXXX-XXXX

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

4. **Verify:**
   - Claude Code does **not** display a 422 or deserialization error
   - The model responds with a description of the image
   - No error messages in Claude Code output

5. **Check adapter logs:**
   ```bash
   cat /tmp/copilot-adapter-image-test.log | grep -i -E "(error|warn|image|multimodal)"
   ```
   Expected:
   - No `Failed to deserialize` errors
   - No `422` status codes
   - Possible debug entries showing multimodal translation

6. **Clean up:**
   ```bash
   copilot-adapter stop
   ```

---

## Test Summary Checklist

| # | Test | Pass/Fail | Notes |
|---|------|-----------|-------|
| 1 | Authentication flow | | |
| 2 | Server start & health | | |
| 3 | Daemon lifecycle | | |
| 4 | Models endpoint | | |
| 4a | Fresh start fetches from API | | |
| 4b | Second request uses cache | | |
| 4c | Request after TTL refetches | | |
| 4d | Network disconnect fallback | | |
| 4e | Static models mode | | |
| 5 | Non-streaming messages | | |
| 6 | Streaming messages | | |
| 7 | Concurrent clients | | |
| 8 | Error handling | | |
| 9 | Logging | | |
| 10 | Claude Code integration | | |
| 11 | Tool call (non-streaming, Anthropic) | | |
| 12 | Tool call (streaming, Anthropic) | | |
| 13 | Multi-turn with tool results | | |
| 14 | Tool call with multiple tools | | |
| 14b | XML tool call format verification | | |
| 15 | Claude Code with tools | | |
| 16 | Image upload (base64) | | |
| 17 | Image upload (URL) | | |
| 18 | Mixed content (image + doc) | | |
| 19 | Image with cache control | | |
| 20 | Claude Code image upload | | |
