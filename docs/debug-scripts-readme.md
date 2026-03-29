# Debug Helper Scripts

These scripts automate the process of building, authenticating, and running the copilot-adapter with comprehensive trace-level logging for debugging tool call issues.

## Files

- **`debug-responses.sh`** - Linux/macOS version
- **`debug-responses.bat`** - Windows version

## What They Do

Both scripts perform these steps automatically:

1. **Build** the adapter in release mode (`cargo build --release`)
2. **Force re-authentication** - Always runs `copilot-adapter auth --force` to ensure fresh credentials
3. **Start with trace logging** - Captures detailed logs to a timestamped file

## Usage

### Linux/macOS

```bash
# Make executable (first time only)
chmod +x debug-responses.sh

# Run
./debug-responses.sh
```

### Windows (Command Prompt)

```cmd
debug-responses.bat
```

### Windows (PowerShell)

```powershell
.\debug-responses.bat
```

## Output

The script creates a log file with timestamp:
```
debug_responses_20260327_235900.log
```

## What Gets Logged

With trace-level logging, you'll see:

### Request Information (DEBUG)
```
DEBUG Received Anthropic messages request model="claude-sonnet-4.5" stream=Some(true) num_messages=3
```

### Tool Injection (DEBUG)
```
DEBUG Injecting Anthropic tools into prompt num_tools=27 tool_names=["WebSearch", "WebFetch", ...]
```

### Streaming Response Content (DEBUG/TRACE)
```
DEBUG Streaming response complete, checking for tool calls content_length=1234
DEBUG Buffered content preview content_preview="I'll search for that information..."
TRACE Full buffered content from streaming response full_content="[entire response]"
```

### Tool Call Parsing (DEBUG)
```
DEBUG Parsed tool calls from streaming response num_tool_calls=1 tool_call_names=["WebSearch"]
```

Or:
```
DEBUG No tool calls found in streaming response
```

## Stopping the Adapter

Press **Ctrl+C** to stop the adapter gracefully.

## After Running

Check the generated log file for:
1. Tool injection confirmation
2. Raw response content from Copilot
3. Tool call parsing results

See **[docs/debugging-tool-calls.md](docs/debugging-tool-calls.md)** for detailed analysis guidance.

## Troubleshooting

### "cargo: command not found"
Install Rust: https://rustup.rs/

### "Authentication failed"
- Ensure you have a GitHub Copilot subscription
- Check your internet connection
- Try running `copilot-adapter auth` manually

### "Build failed"
- Check if you have all dependencies installed (see main README.md)
- Try `cargo clean` then run the script again

### Script won't run (Linux/macOS)
Make it executable:
```bash
chmod +x debug-responses.sh
```

## Manual Alternative

If you prefer manual control:

```bash
# Build
cargo build --release

# Authenticate (if needed)
target/release/copilot-adapter auth

# Start with logging
target/release/copilot-adapter start --log-level trace --log-file my-debug.log
```

## Related Documentation

- **[FINDINGS.md](../FINDINGS.md)** - Investigation findings and what to look for in logs
- **[docs/debugging-tool-calls.md](docs/debugging-tool-calls.md)** - Complete debugging guide
- **[TOOLS-SUPPORT.design.md](../TOOLS-SUPPORT.design.md)** - Tool support design and architecture
