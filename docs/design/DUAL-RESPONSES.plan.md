# Tool Format Migration and Cleanup — Implementation Plan

**Status:** Draft
**Date:** 2026-03-30
**Based on:** [DUAL-RESPONSES.design.md](./DUAL-RESPONSES.design.md)
**Severity:** High

---

## Executive Summary

This plan implements four major changes:

1. **Migrate tool format from JSON to XML** — Adopt LiteLLM/Anthropic Cookbook XML format for better Claude model compatibility
2. **Remove OpenAI `/v1/chat/completions` endpoint** — Focus on native Claude Code (Anthropic API) support only
3. **Document dual-request behavior** — Create known issues documentation
4. **Enhanced debuggability** — Add conversation logging and improve trace output

**Total estimated time:** 5-6 days

---

## Background

### Current State

- `/v1/chat/completions` (OpenAI) and `/v1/messages` (Anthropic) endpoints both exist
- Tool injection uses JSON format: `{"function_call": {"name": "...", "arguments": {...}}}`
- Parser supports JSON (primary) and wrapped XML (fallback)
- Anthropic endpoint internally converts to OpenAI format, then back
- Trace logging exists but is verbose JSON dumps

### Target State

- Only `/v1/messages` (Anthropic) endpoint remains
- Tool injection uses XML format following LiteLLM/Anthropic Cookbook
- Parser uses XML format exclusively
- Cleaner codebase with fewer format conversions
- Human-readable conversation logging option
- Better debug output for tool-related issues

---

## Goals and Non-Goals

### Goals

| ID | Goal | Success Criteria |
|----|------|------------------|
| G1 | Migrate to XML tool format | 95%+ tool calls parsed successfully |
| G2 | Remove OpenAI endpoint | `/v1/chat/completions` returns 404 |
| G3 | Document dual-request behavior | `docs/known-issues.md` exists |
| G4 | Update all documentation | CLAUDE.md, README.md, e2e-testing.md updated |
| G5 | Comprehensive testing | Unit, integration, and E2E tests pass |
| G6 | Conversation logging | `--conversation-log` produces readable summaries |
| G7 | Debug tools mode | `--debug-tools` shows tool flow at INFO level |

### Non-Goals

| ID | Non-Goal | Rationale |
|----|----------|-----------|
| NG1 | Keep JSON format as fallback | Clean break simplifies maintenance |
| NG2 | Keep OpenAI endpoint | Claude Code uses Anthropic API only |
| NG3 | Support `<invoke name="...">` attribute format | Standardize on LiteLLM `<tool_name>` format |

---

## Implementation Plan

### Epic 1: Documentation — Dual Request Behavior (Day 1, 0.5 days)

Document the expected dual-request behavior before making code changes.

#### Task 1.1: Create Known Issues Document

**File:** `docs/known-issues.md` (NEW)

```markdown
# Known Issues

## Multiple Responses from Claude Code

### Description
When using Claude Code through the copilot-adapter, you may see two responses
for a single message. This is expected behavior.

### Cause
Claude Code automatically generates session titles in the background using a
fast, cheap model (e.g., Haiku). This title generation request:
- Uses a different model than your conversation
- Has no conversation history (only sees "Let's implement that", not what "that" refers to)
- Returns a response asking for clarification

### What You'll See
1. A response from Haiku asking "What would you like me to implement?"
2. A response from your selected model (e.g., Sonnet) with the actual answer

### Workaround
Focus on the response from your selected model and ignore the title generator's response.

### Status
This is Claude Code behavior, not an adapter bug. The adapter correctly proxies
all requests it receives.
```

**Acceptance Criteria:**
- [x] File created at `docs/known-issues.md`
- [x] Explains issue clearly
- [x] Provides workaround

#### Task 1.2: Link from README

**File:** `README.md`

Add to the appropriate section:
```markdown
## Known Issues

See [docs/known-issues.md](./docs/known-issues.md) for information about:
- Multiple responses when using Claude Code
```

**Acceptance Criteria:**
- [x] README links to known issues
- [x] Link is in logical location

---

### Epic 2: XML Tool Injector (Day 1-2, 1.5 days) — **DONE**

Replace JSON-based tool injection with XML-based format.

#### Task 2.1: Implement XML Tool Formatter

**File:** `src/tools/injector.rs`

Replace `format_tools_as_json()` with `format_tools_as_xml()`:

```rust
/// Format tool definitions as XML following the Anthropic Cookbook format.
///
/// Output format:
/// ```xml
/// <tools>
/// <tool_description>
/// <tool_name>function_name</tool_name>
/// <description>description text</description>
/// <parameters>
/// <parameter>
/// <name>param_name</name>
/// <type>string</type>
/// <description>param description</description>
/// <required>true</required>
/// </parameter>
/// </parameters>
/// </tool_description>
/// </tools>
/// ```
pub fn format_tools_as_xml(tools: &[Tool]) -> String {
    // Implementation
}

fn format_parameter_xml(name: &str, schema: &serde_json::Value) -> String {
    // Format a single parameter from JSON Schema to XML
}
```

**Acceptance Criteria:**
- [x] `format_tools_as_xml()` implemented
- [x] Handles tools with/without parameters
- [x] Handles required/optional parameters
- [x] Handles various parameter types (string, number, boolean, object, array)
- [x] Unit tests pass

#### Task 2.2: Update Tool Usage Instructions

**File:** `src/tools/injector.rs`

Replace `TOOL_USAGE_INSTRUCTIONS` constant:

```rust
pub const TOOL_USAGE_INSTRUCTIONS: &str = r#"
In this environment you have access to a set of tools you can use to answer the user's question.

You may call them like this:
<function_calls>
<invoke>
<tool_name>$TOOL_NAME</tool_name>
<parameters>
<$PARAMETER_NAME>$PARAMETER_VALUE</$PARAMETER_NAME>
...
</parameters>
</invoke>
</function_calls>

Important rules:
- Always wrap tool calls in <function_calls> tags
- Use <tool_name> to specify which tool to call
- Use <parameters> to pass arguments, with each parameter as <name>value</name>
- You can call multiple tools by including multiple <invoke> blocks
- After receiving tool results, continue your response normally
"#;
```

**Acceptance Criteria:**
- [x] Instructions updated to XML format (attribute-based: `<invoke name="..."><parameter name="...">value</parameter></invoke>`)
- [x] Clear about `<function_calls>` wrapper requirement
- [x] Multiple tool calls explained

#### Task 2.3: Update `build_tool_prompt()`

**File:** `src/tools/injector.rs`

Update to use XML formatter:

```rust
fn build_tool_prompt(tools: &[Tool]) -> String {
    let xml = format_tools_as_xml(tools);
    format!(
        "# Available Functions\n\n{xml}\n{TOOL_USAGE_INSTRUCTIONS}"
    )
}
```

**Acceptance Criteria:**
- [x] Uses XML formatter
- [x] Maintains "Available Functions" header
- [x] Instructions follow tool definitions

#### Task 2.4: Unit Tests for XML Injector

**File:** `src/tools/injector.rs` (inline tests)

Add tests:
- `format_tools_produces_valid_xml` — Basic tool formatting
- `format_tools_with_multiple_params` — Multiple parameters
- `format_tools_with_nested_params` — Object/array parameters
- `format_tools_escapes_special_chars` — `<`, `>`, `&` in descriptions
- `build_tool_prompt_contains_instructions` — Full prompt build

**Acceptance Criteria:**
- [x] All unit tests pass
- [x] Edge cases covered
- [x] Round-trip integration tests added (injected format → model response → parse_tool_calls())
- [x] Duplicate inline tests removed; covered by external test file

**Completion Notes:**
- TOOL_USAGE_INSTRUCTIONS updated to attribute-based XML format (`<invoke name="..."><parameter name="...">value</parameter></invoke>`) matching parser.rs regexes
- parser.rs module doc updated: XML labeled 'primary' (injected format), JSON labeled 'legacy'
- 3 round-trip integration tests added to tests/unit/tools_injector_tests.rs
- Follow-up: module doc vs function doc terminology inconsistency (non-blocking) — 'primary' means different things in each; recommend clarifying in next pass

---

### Epic 3: XML Tool Parser(Day 2-3, 1.5 days)

Replace JSON parsing with XML parsing.

#### Task 3.1: Implement XML Extraction Helpers

**File:** `src/tools/parser.rs`

Add helper functions following LiteLLM's approach:

```rust
/// Extract content between XML tags.
fn extract_between_tags(tag: &str, content: &str) -> Vec<String> {
    let pattern = format!(r"(?s)<{}>(.+?)</{}>", regex::escape(tag), regex::escape(tag));
    let regex = Regex::new(&pattern).expect("tag extraction regex should compile");
    regex.captures_iter(content)
        .filter_map(|cap| cap.get(1).map(|m| m.as_str().to_string()))
        .collect()
}

/// Check if a tag exists in the content.
fn contains_tag(tag: &str, content: &str) -> bool {
    let pattern = format!(r"(?s)<{}>(.+?)</{}>", regex::escape(tag), regex::escape(tag));
    Regex::new(&pattern).unwrap().is_match(content)
}
```

**Acceptance Criteria:**
- [ ] `extract_between_tags()` works with multiline content
- [ ] `contains_tag()` returns correct boolean
- [ ] Handles nested tags correctly

#### Task 3.2: Implement XML Parameter Parser

**File:** `src/tools/parser.rs`

```rust
/// Parse XML parameters into a serde_json::Value object.
///
/// Input format:
/// ```xml
/// <parameters>
/// <file_path>/src/main.rs</file_path>
/// <limit>100</limit>
/// </parameters>
/// ```
///
/// Output: `{"file_path": "/src/main.rs", "limit": "100"}`
fn parse_xml_params(params_content: &str) -> serde_json::Value {
    let mut params = serde_json::Map::new();

    // Match <param_name>value</param_name> patterns
    let param_regex = Regex::new(r"<([a-zA-Z_][a-zA-Z0-9_]*)>([^<]*)</\1>")
        .expect("param regex should compile");

    for cap in param_regex.captures_iter(params_content) {
        let name = cap.get(1).unwrap().as_str();
        let value = cap.get(2).unwrap().as_str().trim();
        params.insert(name.to_string(), serde_json::Value::String(value.to_string()));
    }

    serde_json::Value::Object(params)
}
```

**Acceptance Criteria:**
- [ ] Parses simple parameters
- [ ] Handles whitespace correctly
- [ ] Returns empty object for no parameters

#### Task 3.3: Implement Main XML Parser

**File:** `src/tools/parser.rs`

Replace `parse_tool_calls()`:

```rust
/// Parse tool calls from model-generated text content.
///
/// Primary format (with wrapper):
/// ```xml
/// <function_calls>
/// <invoke>
/// <tool_name>Read</tool_name>
/// <parameters>
/// <file_path>/src/main.rs</file_path>
/// </parameters>
/// </invoke>
/// </function_calls>
/// ```
///
/// Fallback format (standalone invoke):
/// ```xml
/// <invoke>
/// <tool_name>Read</tool_name>
/// <parameters>...</parameters>
/// </invoke>
/// ```
pub fn parse_tool_calls(content: &str) -> Vec<ToolCall> {
    // Primary: Look for <function_calls> wrapper
    if contains_tag("function_calls", content) {
        let fc_content = extract_between_tags("function_calls", content);
        let mut calls = Vec::new();
        for fc in fc_content {
            calls.extend(parse_invokes(&fc));
        }
        if !calls.is_empty() {
            tracing::debug!(num_calls = calls.len(), "Parsed tool calls from <function_calls> blocks");
            return calls;
        }
    }

    // Fallback: Look for standalone <invoke> blocks
    let calls = parse_invokes(content);
    if !calls.is_empty() {
        tracing::debug!(num_calls = calls.len(), "Parsed tool calls from standalone <invoke> blocks");
    }
    calls
}

fn parse_invokes(content: &str) -> Vec<ToolCall> {
    let mut calls = Vec::new();

    for invoke_content in extract_between_tags("invoke", content) {
        if let Some(tc) = try_parse_invoke(&invoke_content) {
            calls.push(tc);
        }
    }

    calls
}

fn try_parse_invoke(invoke_content: &str) -> Option<ToolCall> {
    // Extract tool_name
    let tool_name = extract_between_tags("tool_name", invoke_content)
        .first()
        .map(|s| s.trim().to_string())?;

    if tool_name.is_empty() {
        return None;
    }

    // Extract parameters
    let params = extract_between_tags("parameters", invoke_content)
        .first()
        .map(|s| parse_xml_params(s))
        .unwrap_or(serde_json::Value::Object(serde_json::Map::new()));

    Some(ToolCall {
        id: Some(generate_call_id()),
        call_type: Some("function".to_string()),
        function: FunctionCall {
            name: Some(tool_name),
            arguments: Some(params.to_string()),
        },
    })
}
```

**Acceptance Criteria:**
- [ ] Parses `<function_calls>` wrapped format
- [ ] Falls back to standalone `<invoke>` blocks
- [ ] Extracts `<tool_name>` correctly
- [ ] Parses `<parameters>` correctly
- [ ] Generates unique call IDs
- [ ] Logs parsing results

#### Task 3.4: Update `strip_tool_calls()`

**File:** `src/tools/parser.rs`

Replace JSON stripping with XML stripping:

```rust
/// Strip tool call XML from the content, returning the cleaned text.
pub fn strip_tool_calls(content: &str) -> String {
    let mut result = content.to_string();

    // Remove <function_calls>...</function_calls> blocks
    let fc_regex = Regex::new(r"(?s)<function_calls>.*?</function_calls>")
        .expect("function_calls strip regex should compile");
    result = fc_regex.replace_all(&result, "").to_string();

    // Remove standalone <invoke>...</invoke> blocks
    let invoke_regex = Regex::new(r"(?s)<invoke>.*?</invoke>")
        .expect("invoke strip regex should compile");
    result = invoke_regex.replace_all(&result, "").to_string();

    // Collapse multiple newlines
    let collapse_regex = Regex::new(r"\n{3,}").expect("collapse regex should compile");
    result = collapse_regex.replace_all(&result, "\n\n").to_string();

    result.trim().to_string()
}
```

**Acceptance Criteria:**
- [ ] Removes `<function_calls>` blocks
- [ ] Removes standalone `<invoke>` blocks
- [ ] Preserves surrounding text
- [ ] Collapses extra newlines

#### Task 3.5: Remove JSON Parsing Code

**File:** `src/tools/parser.rs`

Delete:
- `FENCED_PATTERN` regex
- `INLINE_START` regex
- `parse_json_tool_calls()` function
- `try_parse_tool_call()` function (JSON version)
- `find_matching_brace()` helper
- JSON-related unit tests

**Acceptance Criteria:**
- [ ] All JSON parsing code removed
- [ ] No dead code warnings
- [ ] File compiles cleanly

#### Task 3.6: Add Unrecognized Pattern Logging

**File:** `src/tools/parser.rs`

At the end of `parse_tool_calls()`:

```rust
// If no tool calls found but content looks like it might have them, log for debugging
if calls.is_empty() {
    if content.contains("<invoke") || content.contains("<tool") ||
       content.contains("function_call") || content.contains("<function") {
        tracing::warn!(
            content_preview = %content.chars().take(500).collect::<String>(),
            "Content contains tool-like patterns but no valid tool calls were parsed"
        );
    }
}
```

**Acceptance Criteria:**
- [ ] WARN logged when tool-like patterns present but not parsed
- [ ] Content preview included for debugging
- [ ] Normal text doesn't trigger warning

#### Task 3.7: Unit Tests for XML Parser

**File:** `src/tools/parser.rs` (inline tests)

Add comprehensive tests:

```rust
#[test]
fn parse_wrapped_function_calls() {
    let content = r#"
Here's my analysis:

<function_calls>
<invoke>
<tool_name>Read</tool_name>
<parameters>
<file_path>/src/main.rs</file_path>
</parameters>
</invoke>
</function_calls>
"#;
    let calls = parse_tool_calls(content);
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].function.name, Some("Read".to_string()));
}

#[test]
fn parse_standalone_invoke() {
    let content = r#"
<invoke>
<tool_name>Edit</tool_name>
<parameters>
<file_path>/src/lib.rs</file_path>
<old_string>old</old_string>
<new_string>new</new_string>
</parameters>
</invoke>
"#;
    let calls = parse_tool_calls(content);
    assert_eq!(calls.len(), 1);
}

#[test]
fn parse_multiple_invokes() {
    let content = r#"
<function_calls>
<invoke>
<tool_name>Read</tool_name>
<parameters><file_path>/a.rs</file_path></parameters>
</invoke>
<invoke>
<tool_name>Read</tool_name>
<parameters><file_path>/b.rs</file_path></parameters>
</invoke>
</function_calls>
"#;
    let calls = parse_tool_calls(content);
    assert_eq!(calls.len(), 2);
}

#[test]
fn strip_tool_calls_preserves_text() {
    let content = "Before\n<function_calls><invoke><tool_name>X</tool_name><parameters></parameters></invoke></function_calls>\nAfter";
    let stripped = strip_tool_calls(content);
    assert!(stripped.contains("Before"));
    assert!(stripped.contains("After"));
    assert!(!stripped.contains("function_calls"));
}
```

**Acceptance Criteria:**
- [ ] All unit tests pass
- [ ] Edge cases covered (empty params, no wrapper, etc.)
- [ ] Stripping tests included

---

### Epic 4: Remove OpenAI Endpoint (Day 3, 1 day)

Remove the `/v1/chat/completions` endpoint and clean up related code.

#### Task 4.1: Remove Chat Handler

**File:** `src/handlers/chat.rs` — DELETE ENTIRE FILE

**Acceptance Criteria:**
- [ ] File deleted
- [ ] Git tracks deletion

#### Task 4.2: Update Handlers Module

**File:** `src/handlers/mod.rs`

Remove the chat module:

```rust
// Before:
pub mod chat;
pub mod health;
pub mod messages;
pub mod models;

// After:
pub mod health;
pub mod messages;
pub mod models;
```

**Acceptance Criteria:**
- [ ] `chat` module removed
- [ ] Compiles without errors

#### Task 4.3: Update Server Routes

**File:** `src/server.rs`

Remove the `/v1/chat/completions` route:

```rust
// Remove this line:
.route(
    "/v1/chat/completions",
    axum::routing::post(handlers::chat::chat_completions),
)
```

**Acceptance Criteria:**
- [ ] Route removed
- [ ] Server compiles
- [ ] `/v1/chat/completions` returns 404

#### Task 4.4: Clean Up Unused Imports

**Files:** Various

Check and remove unused imports from:
- `src/server.rs`
- `src/handlers/mod.rs`
- Any file that imported from `handlers::chat`

**Acceptance Criteria:**
- [ ] No unused import warnings
- [ ] `cargo clippy` passes

#### Task 4.5: Update Tests

**Files:** `tests/integration/*.rs`

- Remove any integration tests that use `/v1/chat/completions`
- Or update them to use `/v1/messages` instead

**Acceptance Criteria:**
- [ ] All integration tests pass
- [ ] No tests for removed endpoint

---

### Epic 5: Update Messages Handler (Day 3-4, 0.5 days)

Simplify the `/v1/messages` handler now that it's the only endpoint.

#### Task 5.1: Review and Simplify

**File:** `src/handlers/messages.rs`

Review the handler for any code that was only needed to support the OpenAI endpoint. The handler should:

1. Accept Anthropic request
2. Translate to internal format (for Copilot API)
3. Inject tools (XML format)
4. Send to Copilot
5. Parse tool calls (XML format)
6. Translate response back to Anthropic format

No major changes expected, but review for clarity.

**Acceptance Criteria:**
- [ ] Handler reviewed
- [ ] Any unnecessary code removed
- [ ] Comments updated

#### Task 5.2: Update TRACE Logging

**File:** `src/handlers/messages.rs`

Update trace logging messages to reflect that this is now the only endpoint:

```rust
// Update messages like:
// "Full request received from Claude Code (Anthropic format)"
// to be clearer about the flow
```

**Acceptance Criteria:**
- [ ] Log messages are clear
- [ ] Format mentions updated

---

### Epic 6: Documentation Updates (Day 4, 0.5 days)

Update all documentation to reflect the changes.

#### Task 6.1: Update CLAUDE.md

**File:** `CLAUDE.md`

Updates:
1. Remove `/v1/chat/completions` from API Endpoints section
2. Update tool support description to mention XML format
3. Update any references to JSON tool format

```markdown
## API Endpoints

- `GET /health` - Health check
- `POST /v1/messages` - Anthropic-format messages (Claude Code native)
- `GET /v1/models` - List available models
- `GET /v1/models/:model` - Get model details

## Notes for Development

- **Tools/functions support** is always enabled — tool definitions are injected
  into the system prompt using XML format (following the Anthropic Cookbook);
  tool calls are parsed from `<function_calls>` XML blocks in model responses
```

**Acceptance Criteria:**
- [ ] API Endpoints updated
- [ ] Tool format description updated
- [ ] No references to OpenAI endpoint

#### Task 6.2: Update README.md

**File:** `README.md`

Updates:
1. Update API section
2. Add link to known issues
3. Remove any OpenAI endpoint examples

**Acceptance Criteria:**
- [ ] README accurate
- [ ] Known issues linked

#### Task 6.3: Update E2E Testing Doc

**File:** `docs/e2e-testing.md`

Updates:
1. Remove any `/v1/chat/completions` test procedures
2. Update tool format expectations (XML instead of JSON)
3. Add test case for XML tool calls

**Acceptance Criteria:**
- [ ] Testing procedures accurate
- [ ] XML format documented

#### Task 6.4: Update TOOLS-SUPPORT Documents

**Files:**
- `docs/design/TOOLS-SUPPORT.design.md`
- `docs/design/TOOLS-SUPPORT.plan.md`

Add deprecation note at the top:

```markdown
**Status:** Deprecated (superseded by DUAL-RESPONSES.design.md)
**Note:** The JSON tool format described in this document has been replaced
with XML format following the Anthropic Cookbook. See DUAL-RESPONSES.design.md
for current implementation.
```

**Acceptance Criteria:**
- [ ] Deprecation notes added
- [ ] References to new design doc

---

### Epic 7: Integration Testing (Day 5, 1 day)

Comprehensive testing of all changes.

#### Task 7.1: Update Integration Tests

**Files:** `tests/integration/*.rs`

Update tests to:
1. Use only `/v1/messages` endpoint
2. Expect XML tool format in mock responses
3. Verify XML tool injection in requests

**Acceptance Criteria:**
- [ ] All integration tests updated
- [ ] Tests pass

#### Task 7.2: Create XML-Specific Tests

**File:** `tests/integration/tools_xml_tests.rs` (NEW)

Add integration tests specifically for XML format:
- Tool injection produces valid XML
- Mock response with XML tool calls parsed correctly
- Multiple tool calls work
- Streaming with tools works

**Acceptance Criteria:**
- [ ] New test file created
- [ ] Covers XML-specific scenarios

#### Task 7.3: Manual E2E Testing

Execute manual tests with actual Claude Code:

1. **Basic tool call**: Ask Claude to read a file
2. **Multi-tool call**: Ask for a change that requires read + edit
3. **Streaming**: Verify streaming responses work
4. **Error cases**: Malformed tool calls degrade gracefully

**Acceptance Criteria:**
- [ ] All manual tests pass
- [ ] Results documented

#### Task 7.4: Regression Testing

Run full test suite:

```bash
cargo test
cargo clippy
cargo fmt --check
```

**Acceptance Criteria:**
- [ ] All tests pass
- [ ] No clippy warnings
- [ ] Code formatted

---

### Epic 8: Enhanced Debuggability (Day 5-6, 1 day)

Add human-readable conversation logging and improved debug output.

#### Task 8.1: Add CLI Flags

**File:** `src/cli.rs`

Add new flags to the `Start` command:

```rust
/// Path to write human-readable conversation logs
#[arg(long)]
conversation_log: Option<String>,

/// Maximum size for conversation log before rotation (bytes, default: 10MB)
#[arg(long, default_value_t = 10_485_760)]
conversation_log_max_size: u64,

/// Enable verbose tool-related logging at INFO level
#[arg(long)]
debug_tools: bool,
```

**Acceptance Criteria:**
- [ ] Flags added to CLI
- [ ] Help text is clear
- [ ] Flags parsed correctly

#### Task 8.2: Create Conversation Log Module

**File:** `src/conversation_log.rs` (NEW)

Create a module for human-readable conversation logging:

```rust
use std::path::Path;
use tokio::fs::OpenOptions;
use tokio::io::AsyncWriteExt;

pub struct ConversationLogger {
    path: std::path::PathBuf,
    max_size: u64,
    request_counter: std::sync::atomic::AtomicU64,
}

impl ConversationLogger {
    pub fn new(path: impl AsRef<Path>, max_size: u64) -> Self { ... }

    /// Log a complete request/response cycle
    pub async fn log_cycle(&self, cycle: &ConversationCycle) -> std::io::Result<()> { ... }

    /// Rotate log file if it exceeds max size
    async fn maybe_rotate(&self) -> std::io::Result<()> { ... }
}

pub struct ConversationCycle {
    pub timestamp: chrono::DateTime<chrono::Utc>,
    pub request_id: String,

    // From Claude Code
    pub incoming_model: String,
    pub incoming_stream: bool,
    pub incoming_messages: Vec<MessageSummary>,
    pub incoming_system: Option<String>,
    pub incoming_tools: Vec<String>,

    // To Copilot
    pub outgoing_model: String,
    pub outgoing_messages_count: usize,
    pub tools_injected: bool,
    pub xml_injection_size: usize,

    // From Copilot
    pub response_model: String,
    pub response_finish_reason: Option<String>,
    pub response_content_preview: String,
    pub response_has_tool_calls: bool,

    // To Claude Code
    pub final_stop_reason: Option<String>,
    pub final_content_blocks: Vec<ContentBlockSummary>,
    pub parsed_tool_calls: Vec<ToolCallSummary>,
}

pub struct MessageSummary {
    pub role: String,
    pub content_preview: String,
    pub content_length: usize,
    pub has_tool_use: bool,
    pub has_tool_result: bool,
}

pub struct ContentBlockSummary {
    pub block_type: String,
    pub preview: String,
}

pub struct ToolCallSummary {
    pub id: String,
    pub name: String,
    pub arguments_preview: String,
}
```

**Acceptance Criteria:**
- [ ] Module compiles
- [ ] Async file writes work
- [ ] Log rotation works

#### Task 8.3: Implement Summary Formatters

**File:** `src/conversation_log.rs`

Implement the formatting logic:

```rust
impl ConversationCycle {
    pub fn format(&self) -> String {
        let mut output = String::new();

        // Header
        output.push_str(&"=".repeat(80));
        output.push_str(&format!("\n[{}] Request #{}\n", self.timestamp, self.request_id));
        output.push_str(&"=".repeat(80));
        output.push('\n');

        // From Claude Code section
        output.push_str("\n>>> FROM CLAUDE CODE (Anthropic format)\n");
        output.push_str(&format!("Model: {}\n", self.incoming_model));
        output.push_str(&format!("Stream: {}\n", self.incoming_stream));
        output.push_str(&format!("Messages: {}\n", self.incoming_messages.len()));
        // ... more formatting

        // To Copilot section
        output.push_str("\n");
        output.push_str(&"-".repeat(80));
        output.push_str("\n>>> TO GITHUB COPILOT API (OpenAI format)\n");
        // ... more formatting

        // From Copilot section
        output.push_str("\n");
        output.push_str(&"-".repeat(80));
        output.push_str("\n<<< FROM GITHUB COPILOT API (OpenAI format)\n");
        // ... more formatting

        // To Claude Code section
        output.push_str("\n");
        output.push_str(&"-".repeat(80));
        output.push_str("\n<<< TO CLAUDE CODE (Anthropic format)\n");
        // ... more formatting

        output.push_str("\n");
        output.push_str(&"=".repeat(80));
        output.push('\n');

        output
    }
}
```

**Acceptance Criteria:**
- [ ] Output is human-readable
- [ ] Content is appropriately truncated (previews)
- [ ] All four sections present
- [ ] Tool information clearly shown

#### Task 8.4: Integrate Logger into Messages Handler

**File:** `src/handlers/messages.rs`

Add conversation logging at the end of request processing:

```rust
// At the start of the handler
let cycle_builder = if let Some(ref logger) = state.conversation_logger {
    Some(ConversationCycleBuilder::new(request_id.clone()))
} else {
    None
};

// Capture incoming request info
if let Some(ref mut builder) = cycle_builder {
    builder.set_incoming(&request);
}

// ... existing processing ...

// Capture outgoing to Copilot
if let Some(ref mut builder) = cycle_builder {
    builder.set_outgoing(&openai_request);
}

// ... send to Copilot ...

// Capture response from Copilot
if let Some(ref mut builder) = cycle_builder {
    builder.set_copilot_response(&response);
}

// ... process response ...

// Capture final response to Claude Code
if let Some(ref mut builder) = cycle_builder {
    builder.set_final(&anthropic_response, &parsed_tool_calls);
    let cycle = builder.build();
    if let Some(ref logger) = state.conversation_logger {
        // Non-blocking log write
        let logger = logger.clone();
        tokio::spawn(async move {
            if let Err(e) = logger.log_cycle(&cycle).await {
                tracing::warn!(error = %e, "Failed to write conversation log");
            }
        });
    }
}
```

**Acceptance Criteria:**
- [ ] Logger integrated without blocking request
- [ ] All four phases captured
- [ ] Works for both streaming and non-streaming

#### Task 8.5: Add Debug Tools Mode

**File:** `src/tools/injector.rs`, `src/tools/parser.rs`

Add conditional INFO-level logging when `debug_tools` is enabled:

```rust
// In injector.rs
pub fn inject_tools_into_messages(
    messages: &mut Vec<Message>,
    tools: &[Tool],
    debug_tools: bool,
) {
    if tools.is_empty() {
        return;
    }

    let tool_prompt = build_tool_prompt(tools);

    if debug_tools {
        tracing::info!(
            num_tools = tools.len(),
            tool_names = ?tools.iter().map(|t| &t.function.name).collect::<Vec<_>>(),
            xml_size = tool_prompt.len(),
            xml_preview = %tool_prompt.chars().take(500).collect::<String>(),
            "DEBUG_TOOLS: Injecting tools into system prompt"
        );
    }

    // ... rest of injection
}

// In parser.rs
pub fn parse_tool_calls(content: &str, debug_tools: bool) -> Vec<ToolCall> {
    // ... parsing ...

    if debug_tools {
        if calls.is_empty() {
            tracing::info!(
                content_length = content.len(),
                has_invoke = contains_tag("invoke", content),
                has_tool_name = contains_tag("tool_name", content),
                has_function_calls = contains_tag("function_calls", content),
                content_preview = %content.chars().take(300).collect::<String>(),
                "DEBUG_TOOLS: No tool calls parsed from response"
            );
        } else {
            tracing::info!(
                num_calls = calls.len(),
                tool_names = ?calls.iter().map(|tc| &tc.function.name).collect::<Vec<_>>(),
                "DEBUG_TOOLS: Successfully parsed tool calls"
            );
        }
    }

    calls
}
```

**Acceptance Criteria:**
- [ ] `--debug-tools` produces useful INFO logs
- [ ] Logs show injection details
- [ ] Logs show parsing results
- [ ] No overhead when flag not set

#### Task 8.6: Update AppState

**File:** `src/server.rs`

Add conversation logger to AppState:

```rust
pub struct AppState {
    pub token_manager: Arc<TokenManager>,
    pub http_client: reqwest::Client,
    pub copilot_client: CopilotClient,
    pub config: AdapterConfig,
    pub models_cache: ModelsCache,
    pub conversation_logger: Option<Arc<ConversationLogger>>,  // NEW
    pub debug_tools: bool,  // NEW
}
```

Update `AdapterConfig`:

```rust
pub struct AdapterConfig {
    pub static_models: bool,
    pub models_cache_ttl: std::time::Duration,
    pub conversation_log_path: Option<std::path::PathBuf>,  // NEW
    pub conversation_log_max_size: u64,  // NEW
    pub debug_tools: bool,  // NEW
}
```

**Acceptance Criteria:**
- [ ] AppState updated
- [ ] Config passed through correctly
- [ ] Logger initialized on startup if path provided

#### Task 8.7: Update Trace Logging Documentation

**File:** `docs/design/TRACE-LOGGING.md`

Add section about conversation logging and debug-tools mode:

```markdown
## Conversation Logging

For easier debugging, the adapter can write human-readable conversation summaries:

```bash
copilot-adapter start --conversation-log /tmp/conversations.log
```

This produces a readable log showing:
- What Claude Code sent
- How it was transformed for Copilot
- What Copilot responded
- How the response was transformed back
- Tool injection and parsing details

### Debug Tools Mode

For tool-specific debugging without full trace logs:

```bash
copilot-adapter start --debug-tools
```

This logs (at INFO level):
- Tool definitions being injected
- XML injection size and preview
- Tool call parsing results
- Parse failures with diagnostic info
```

**Acceptance Criteria:**
- [ ] Documentation updated
- [ ] Examples provided
- [ ] Clear explanation of when to use each option

#### Task 8.8: Unit Tests for Conversation Logger

**File:** `src/conversation_log.rs` or `tests/unit/conversation_log_tests.rs`

Add tests:
- `format_produces_readable_output`
- `log_rotation_works`
- `async_write_completes`
- `handles_empty_messages`
- `truncates_long_content`

**Acceptance Criteria:**
- [ ] All tests pass
- [ ] Edge cases covered

---

## Timeline Summary

| Epic | Description | Duration | Dependencies |
|------|-------------|----------|--------------|
| Epic 1 | Documentation (dual-request) | 0.5 days | None |
| Epic 2 | XML Tool Injector | 1.5 days | None |
| Epic 3 | XML Tool Parser | 1.5 days | Epic 2 (for testing) |
| Epic 4 | Remove OpenAI Endpoint | 1 day | None |
| Epic 5 | Update Messages Handler | 0.5 days | Epic 4 |
| Epic 6 | Documentation Updates | 0.5 days | All above |
| Epic 7 | Integration Testing | 1 day | All above |
| Epic 8 | Enhanced Debuggability | 1 day | Epics 2, 3 |

**Total: 5-6 days**

---

## Rollback Plan

If critical issues arise:

1. **Revert XML changes**: Restore JSON injector and parser from git history
2. **Restore OpenAI endpoint**: Restore `chat.rs` and server routes
3. **Document issues**: Update known issues doc with findings

All changes are additive then subtractive, making rollback straightforward via git.

---

## Success Metrics

| Metric | Target |
|--------|--------|
| Tool call parse success rate | ≥95% |
| Integration test pass rate | 100% |
| Manual E2E test pass rate | 100% |
| Code coverage (tools module) | ≥80% |
| Build warnings | 0 |
| Conversation log readability | Human can understand flow in <30 seconds |

---

## Checklist Summary

### Epic 1: Documentation
- [x] `docs/known-issues.md` created
- [x] README links to known issues

### Epic 2: XML Injector
- [ ] `format_tools_as_xml()` implemented
- [ ] `TOOL_USAGE_INSTRUCTIONS` updated
- [ ] Unit tests pass

### Epic 3: XML Parser
- [ ] `extract_between_tags()` implemented
- [ ] `parse_xml_params()` implemented
- [ ] `parse_tool_calls()` uses XML
- [ ] `strip_tool_calls()` uses XML
- [ ] JSON code removed
- [ ] Unrecognized pattern logging added
- [ ] Unit tests pass

### Epic 4: Remove OpenAI Endpoint
- [ ] `src/handlers/chat.rs` deleted
- [ ] Route removed from server
- [ ] Unused imports cleaned up

### Epic 5: Messages Handler
- [ ] Handler reviewed and simplified

### Epic 6: Documentation
- [ ] CLAUDE.md updated
- [ ] README.md updated
- [ ] e2e-testing.md updated
- [ ] TOOLS-SUPPORT docs marked deprecated

### Epic 7: Testing
- [ ] Integration tests updated
- [ ] Manual E2E tests pass
- [ ] Full test suite passes

### Epic 8: Debuggability
- [ ] `--conversation-log` flag added
- [ ] `--conversation-log-max-size` flag added
- [ ] `--debug-tools` flag added
- [ ] `ConversationLogger` module created
- [ ] Human-readable format implemented
- [ ] Logger integrated into messages handler
- [ ] Debug tools mode in injector/parser
- [ ] TRACE-LOGGING.md updated
- [ ] Unit tests for logger pass

---

## References

- [DUAL-RESPONSES.design.md](./DUAL-RESPONSES.design.md) — Design document
- [LiteLLM Source](https://github.com/BerriAI/litellm) — Reference implementation
- [Anthropic Function Calling Cookbook](https://github.com/anthropics/anthropic-cookbook/blob/main/function_calling/function_calling.ipynb)
- `TOOLS-SUPPORT.design.md` — Original JSON implementation (deprecated)
