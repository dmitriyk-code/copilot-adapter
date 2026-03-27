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

**Purpose:** Verify the `/v1/models` endpoints return valid model listings.

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

---

## Test 5: Non-Streaming Chat Completion

**Purpose:** Verify chat completions work in non-streaming mode.

### Steps

1. **Send a simple request:**
   ```bash
   curl -s -X POST http://127.0.0.1:6767/v1/chat/completions \
     -H "Content-Type: application/json" \
     -d '{
       "model": "gpt-4",
       "messages": [{"role": "user", "content": "Say hello in one sentence."}]
     }' | python3 -m json.tool
   ```

2. **Expected response format:**
   ```json
   {
     "id": "chatcmpl-...",
     "object": "chat.completion",
     "created": 1234567890,
     "model": "gpt-4",
     "choices": [
       {
         "index": 0,
         "message": {
           "role": "assistant",
           "content": "Hello! How can I assist you today?"
         },
         "finish_reason": "stop"
       }
     ],
     "usage": {
       "prompt_tokens": 12,
       "completion_tokens": 8,
       "total_tokens": 20
     }
   }
   ```

3. **Verify:**
   - Response has valid JSON structure
   - `object` is `"chat.completion"`
   - `choices` array has at least one entry
   - `message.role` is `"assistant"`
   - `message.content` is non-empty

### With System Message

```bash
curl -s -X POST http://127.0.0.1:6767/v1/chat/completions \
  -H "Content-Type: application/json" \
  -d '{
    "model": "gpt-4",
    "messages": [
      {"role": "system", "content": "You are a helpful assistant. Respond in exactly 3 words."},
      {"role": "user", "content": "What is Rust?"}
    ]
  }' | python3 -m json.tool
```

---

## Test 6: Streaming Chat Completion

**Purpose:** Verify SSE streaming returns proper Server-Sent Events.

### Steps

1. **Send a streaming request:**
   ```bash
   curl -N -X POST http://127.0.0.1:6767/v1/chat/completions \
     -H "Content-Type: application/json" \
     -d '{
       "model": "gpt-4",
       "messages": [{"role": "user", "content": "Count from 1 to 5."}],
       "stream": true
     }'
   ```

2. **Expected output format:**
   ```
   data: {"id":"chatcmpl-...","object":"chat.completion.chunk","created":...,"model":"gpt-4","choices":[{"index":0,"delta":{"role":"assistant"},"finish_reason":null}]}

   data: {"id":"chatcmpl-...","object":"chat.completion.chunk","created":...,"model":"gpt-4","choices":[{"index":0,"delta":{"content":"1"},"finish_reason":null}]}

   ...

   data: [DONE]
   ```

3. **Verify:**
   - Each line starts with `data: `
   - Frames separated by double newlines (`\n\n`)
   - Each chunk has `object: "chat.completion.chunk"`
   - First chunk may carry `delta.role: "assistant"`
   - Subsequent chunks carry `delta.content` with text fragments
   - Stream ends with `data: [DONE]`

---

## Test 7: Concurrent Clients

**Purpose:** Verify the adapter handles multiple simultaneous requests.

### Steps

1. **Start the adapter.**

2. **Open 5 terminal windows and run simultaneously:**

   Terminal 1:
   ```bash
   curl -N -X POST http://127.0.0.1:6767/v1/chat/completions \
     -H "Content-Type: application/json" \
     -d '{"model": "gpt-4", "messages": [{"role": "user", "content": "Terminal 1"}], "stream": true}'
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
  curl -s -X POST http://127.0.0.1:6767/v1/chat/completions \
    -H "Content-Type: application/json" \
    -d "{\"model\": \"gpt-4\", \"messages\": [{\"role\": \"user\", \"content\": \"Request $i\"}]}" \
    -o "/tmp/copilot-test-$i.json" &
done
wait

# Check all responses
for i in $(seq 1 10); do
  echo "Request $i: $(python3 -c "import json; d=json.load(open('/tmp/copilot-test-$i.json')); print(d.get('object', 'ERROR'))")"
done
```

---

## Test 8: Error Handling

**Purpose:** Verify proper error responses for invalid inputs.

### Empty Messages

```bash
curl -s -w "\nHTTP Status: %{http_code}\n" -X POST http://127.0.0.1:6767/v1/chat/completions \
  -H "Content-Type: application/json" \
  -d '{"model": "gpt-4", "messages": []}'
```
Expected: HTTP 400, OpenAI error format with `type: "invalid_request_error"`.

### Invalid JSON

```bash
curl -s -w "\nHTTP Status: %{http_code}\n" -X POST http://127.0.0.1:6767/v1/chat/completions \
  -H "Content-Type: application/json" \
  -d 'not valid json'
```
Expected: HTTP 400 or 422.

### Unauthenticated (After Logout)

```bash
copilot-adapter logout
copilot-adapter start
# In another terminal:
curl -s -w "\nHTTP Status: %{http_code}\n" -X POST http://127.0.0.1:6767/v1/chat/completions \
  -H "Content-Type: application/json" \
  -d '{"model": "gpt-4", "messages": [{"role": "user", "content": "Hello"}]}'
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
   - `Sending chat completion request to Copilot API`

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
   export OPENAI_API_BASE=http://127.0.0.1:6767/v1
   export OPENAI_API_KEY=dummy
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

## Test 11: Tool Call (Non-Streaming, OpenAI Format)

**Purpose:** Verify tool/function calling works with `--experimental-tools` enabled.

> **Prerequisites:**
> - Adapter started with `--experimental-tools` flag
> - Authenticated with GitHub

### Steps

1. **Start the adapter with tools enabled:**
   ```bash
   copilot-adapter start --experimental-tools
   ```

2. **Send a request with tool definitions:**
   ```bash
   curl -s -X POST http://127.0.0.1:6767/v1/chat/completions \
     -H "Content-Type: application/json" \
     -d '{
       "model": "gpt-4",
       "messages": [{"role": "user", "content": "What directory am I in?"}],
       "tools": [{
         "type": "function",
         "function": {
           "name": "bash",
           "description": "Run a bash command",
           "parameters": {
             "type": "object",
             "properties": {
               "command": {"type": "string", "description": "The command to run"}
             },
             "required": ["command"]
           }
         }
       }]
     }' | python3 -m json.tool
   ```

3. **Expected response:**
   - `choices[0].message.tool_calls` should be an array with at least one tool call
   - Each tool call should have `id` (starting with `call_`), `type: "function"`, and `function.name`
   - `finish_reason` should be `"tool_calls"`
   - The fenced JSON block should be stripped from `content`

### Verification

- Response has valid `tool_calls` array
- Arguments are valid JSON
- Content does not contain ````json` blocks

---

## Test 12: Tool Call (Streaming)

**Purpose:** Verify tool calls are detected in streaming responses.

### Steps

1. **Send a streaming request with tools:**
   ```bash
   curl -N -X POST http://127.0.0.1:6767/v1/chat/completions \
     -H "Content-Type: application/json" \
     -d '{
       "model": "gpt-4",
       "messages": [{"role": "user", "content": "List files in the current directory"}],
       "stream": true,
       "tools": [{
         "type": "function",
         "function": {
           "name": "bash",
           "description": "Run a bash command",
           "parameters": {
             "type": "object",
             "properties": {
               "command": {"type": "string"}
             },
             "required": ["command"]
           }
         }
       }]
     }'
   ```

2. **Expected output:**
   - SSE events with text content chunks (no fenced JSON)
   - A chunk with `delta.tool_calls` containing the parsed tool call
   - The tool call chunk should have `finish_reason: "tool_calls"`
   - Stream ends with `data: [DONE]`

---

## Test 13: Multi-Turn Conversation with Tool Results

**Purpose:** Verify the adapter handles tool result messages in follow-up requests.

### Steps

1. **Send a request with a tool result from a previous turn:**
   ```bash
   curl -s -X POST http://127.0.0.1:6767/v1/chat/completions \
     -H "Content-Type: application/json" \
     -d '{
       "model": "gpt-4",
       "messages": [
         {"role": "user", "content": "What directory am I in?"},
         {"role": "assistant", "content": "Let me check.", "tool_calls": [{"id": "call_123", "type": "function", "function": {"name": "bash", "arguments": "{\"command\":\"pwd\"}"}}]},
         {"role": "tool", "content": "/home/user/project", "tool_call_id": "call_123"}
       ],
       "tools": [{
         "type": "function",
         "function": {
           "name": "bash",
           "description": "Run a bash command",
           "parameters": {"type": "object", "properties": {"command": {"type": "string"}}, "required": ["command"]}
         }
       }]
     }' | python3 -m json.tool
   ```

2. **Expected response:**
   - The model should receive the tool result and generate a follow-up response
   - The `tool` role message should be translated internally to a `user` role message
   - Response should be valid JSON

---

## Test 14: Tools Disabled (Rejection)

**Purpose:** Verify requests with tools are rejected when `--experimental-tools` is not set.

### Steps

1. **Start the adapter WITHOUT the tools flag:**
   ```bash
   copilot-adapter start
   ```

2. **Send a request with tools:**
   ```bash
   curl -s -w "\nHTTP Status: %{http_code}\n" -X POST http://127.0.0.1:6767/v1/chat/completions \
     -H "Content-Type: application/json" \
     -d '{
       "model": "gpt-4",
       "messages": [{"role": "user", "content": "Hello"}],
       "tools": [{"type": "function", "function": {"name": "test", "parameters": {"type": "object"}}}]
     }'
   ```

3. **Expected response:**
   - HTTP 400 status code
   - Error message mentioning `--experimental-tools`

---

## Test 15: Tool Call (Anthropic Format)

**Purpose:** Verify tool support via the `/v1/messages` endpoint.

### Steps

1. **Start with tools enabled** (if not already running).

2. **Send an Anthropic-format request with tools:**
   ```bash
   curl -s -X POST http://127.0.0.1:6767/v1/messages \
     -H "Content-Type: application/json" \
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
             "command": {"type": "string"}
           },
           "required": ["command"]
         }
       }]
     }' | python3 -m json.tool
   ```

3. **Expected response:**
   - `content` array should contain a `tool_use` block with `name`, `id`, and `input`
   - `stop_reason` should be `"tool_use"`
   - Text blocks should not contain fenced JSON

---

## Test 16: Claude Code with Tools Integration

**Purpose:** Verify Claude Code's native tool use works through the adapter.

### Steps

1. **Start the adapter with tools enabled:**
   ```bash
   copilot-adapter start --daemon --experimental-tools
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

## Test Summary Checklist

| # | Test | Pass/Fail | Notes |
|---|------|-----------|-------|
| 1 | Authentication flow | | |
| 2 | Server start & health | | |
| 3 | Daemon lifecycle | | |
| 4 | Models endpoint | | |
| 5 | Non-streaming chat | | |
| 6 | Streaming chat | | |
| 7 | Concurrent clients | | |
| 8 | Error handling | | |
| 9 | Logging | | |
| 10 | Claude Code integration | | |
| 11 | Tool call (non-streaming) | | |
| 12 | Tool call (streaming) | | |
| 13 | Multi-turn with tool results | | |
| 14 | Tools disabled rejection | | |
| 15 | Tool call (Anthropic format) | | |
| 16 | Claude Code with tools | | |
