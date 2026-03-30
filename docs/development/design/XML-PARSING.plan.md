# XML Tool Call Parsing Support — Implementation Plan

**Status:** DRAFT
**Date:** 2026-03-30
**Related:** [TOOLS-SUPPORT.plan.md](./TOOLS-SUPPORT.plan.md), [REGRESSION-FIX.md](./REGRESSION-FIX.md)
**Issue:** Parser only recognizes JSON format; Claude models generate XML format

---

## Executive Summary

This plan adds XML parsing support to the tool call parser to handle Claude models generating `<function_calls>` XML format instead of the JSON format requested via prompt injection.

**Current behavior:**
- Parser looks for JSON: `{"function_call": {"name": "...", "arguments": {...}}}`
- Claude models generate XML: `<function_calls><invoke name="..."><parameter ...></invoke></function_calls>`
- Parser returns empty → XML returned as plain text → Claude Code displays XML instead of executing tools

**Proposed fix:**
- Add XML parsing to `src/tools/parser.rs`
- Support both JSON and XML formats
- No new dependencies (use existing `regex` crate)
- Backwards compatible

---

## Background

### Current State

The tool calling implementation (from TOOLS-SUPPORT.plan.md) uses **prompt injection**:

1. Tool definitions injected into system prompt as JSON
2. Model instructed to respond with: `{"function_call": {"name": "X", "arguments": {...}}}`
3. Parser (`src/tools/parser.rs`) extracts JSON tool calls
4. Converts to Anthropic `tool_use` blocks for Claude Code

**Parser supports:**
- Fenced JSON blocks: ` ```json\n{"function_call": ...}\n``` `
- Inline JSON: `{"function_call": ...}`
- Multiple tool calls
- Graceful handling of malformed JSON

### Problem Statement

When using Claude models via Copilot, the model sometimes ignores JSON format instructions and generates its native XML format:

```xml
<function_calls>
<invoke name="Bash">
<parameter name="command">ls -la</parameter>
<parameter name="description">List files</parameter>
</invoke>
</function_calls>
```

**Log evidence:**
```
TRACE Buffered content preview (truncated) content_preview=<function_calls>...
DEBUG No tool calls found in streaming response
```

**Result:**
- Parser fails to find tool calls
- Returns `{"type": "text", "text": "<function_calls>..."}` to Claude Code
- Claude Code displays XML as plain text instead of executing tools
- User sees XML in the UI instead of tool execution

### Root Cause

Claude models have strong training to use `<function_calls>` XML format. Even with JSON instructions in the prompt, they may revert to XML. This appears to be happening more frequently recently.

---

## Goals and Non-Goals

### Goals

| ID | Goal | Success Criteria |
|----|------|------------------|
| G1 | Parse XML tool calls | `<function_calls>` format correctly extracted |
| G2 | Maintain JSON parsing | All existing JSON tests still pass |
| G3 | No new dependencies | Use existing `regex` crate |
| G4 | Backwards compatible | No breaking changes to API or existing behavior |
| G5 | Comprehensive testing | Both formats tested; edge cases covered |

### Non-Goals

| ID | Non-Goal | Rationale |
|----|----------|-----------|
| NG1 | Support arbitrary XML | Only Claude's specific `<function_calls>` format |
| NG2 | Add XML library dependency | Regex sufficient for simple, predictable structure |
| NG3 | Change prompt injection | Keep JSON instructions; add XML parsing as fallback |

---

## Requirements

### Functional Requirements

| ID | Requirement | Source |
|----|-------------|--------|
| FR1 | Parse `<function_calls>` XML blocks | User logs showing XML in responses |
| FR2 | Extract `<invoke name="X">` tool name | Claude Code XML format |
| FR3 | Extract `<parameter name="K">V</parameter>` pairs | Claude Code XML format |
| FR4 | Convert XML to `ToolCall` struct | Existing parser output format |
| FR5 | Try JSON parsing first, XML as fallback | Performance and backwards compatibility |
| FR6 | Strip XML tool calls from content | Same as JSON stripping behavior |
| FR7 | Handle multiple `<invoke>` blocks | Claude can generate multiple tool calls |
| FR8 | Gracefully handle malformed XML | Return empty on parse failure |

### Non-Functional Requirements

| ID | Requirement | Target |
|----|-------------|--------|
| NFR1 | XML parsing latency | < 10ms (same as JSON parsing) |
| NFR2 | No new dependencies | Use only existing crates |
| NFR3 | Test coverage | > 90% for new XML code paths |

---

## Proposed Architecture

### XML Format Specification

Based on user logs and Claude Code's format:

```xml
<function_calls>
  <invoke name="ToolName">
    <parameter name="param1">value1</parameter>
    <parameter name="param2">value2</parameter>
  </invoke>
  <invoke name="AnotherTool">
    <parameter name="key">value</parameter>
  </invoke>
</function_calls>
```

**Mapping to `ToolCall`:**
- `<invoke name="X">` → `ToolCall.function.name = "X"`
- Each `<parameter name="K">V</parameter>` → Add `{"K": "V"}` to arguments JSON object
- Generate unique ID using existing `generate_call_id()`
- Serialize parameters as JSON string: `'{"param1": "value1", "param2": "value2"}'`

### Component Changes

**File:** `src/tools/parser.rs`

```
Current structure:
├── parse_tool_calls(content) → Vec<ToolCall>  [PUBLIC API]
├── strip_tool_calls(content) → String         [PUBLIC API]
├── try_parse_tool_call(json) → Option<ToolCall>
├── find_matching_brace(...)
├── generate_call_id() → String
└── is_overlapping(...)

New structure:
├── parse_tool_calls(content) → Vec<ToolCall>  [PUBLIC API - MODIFIED]
│   ├── parse_json_tool_calls(content)         [NEW - extracted from current logic]
│   └── parse_xml_tool_calls(content)          [NEW]
├── strip_tool_calls(content) → String         [PUBLIC API - MODIFIED]
│   ├── strip_json_tool_calls(content)         [implicit in existing logic]
│   └── strip_xml_tool_calls(content)          [NEW]
├── try_parse_tool_call(json) → Option<ToolCall>
├── try_parse_xml_invoke(xml) → Option<ToolCall>  [NEW]
├── find_matching_brace(...)
├── generate_call_id() → String
└── is_overlapping(...)
```

### Regex Patterns

Add new static regexes (similar to existing `FENCED_PATTERN`, `INLINE_START`):

```rust
/// Matches <function_calls>...</function_calls> blocks
static XML_FUNCTION_CALLS: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"(?s)<function_calls>(.*?)</function_calls>")
        .expect("xml function_calls regex should compile")
});

/// Matches <invoke name="...">...</invoke> blocks
static XML_INVOKE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r#"(?s)<invoke\s+name="([^"]+)">(.*?)</invoke>"#)
        .expect("xml invoke regex should compile")
});

/// Matches <parameter name="...">...</parameter>
static XML_PARAMETER: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r#"<parameter\s+name="([^"]+)">([^<]*)</parameter>"#)
        .expect("xml parameter regex should compile")
});
```

---

## Implementation Plan

### Epic 1: Extract JSON Parsing Logic

**Goal:** Refactor existing code to make room for XML parsing

**Tasks:**

1. **E1-T1:** Create `parse_json_tool_calls()` function
   - Move existing `parse_tool_calls()` logic into new function
   - Return `Vec<ToolCall>`
   - No behavior changes

2. **E1-T2:** Update `parse_tool_calls()` to call `parse_json_tool_calls()`
   - Simple wrapper for now
   - Maintains API compatibility
   - All existing tests should still pass

**Acceptance Criteria:**
- All existing unit tests pass unchanged
- No functional changes
- Code is cleaner and ready for XML parsing

---

### Epic 2: Add XML Parsing

**Goal:** Implement XML tool call parsing

**Tasks:**

1. **E2-T1:** Add XML regex patterns
   - Define `XML_FUNCTION_CALLS`, `XML_INVOKE`, `XML_PARAMETER` static regexes
   - Test patterns in isolation (inline tests)

2. **E2-T2:** Implement `parse_xml_tool_calls()`
   - Find `<function_calls>` blocks with `XML_FUNCTION_CALLS`
   - Extract `<invoke>` blocks with `XML_INVOKE`
   - For each invoke: extract parameters with `XML_PARAMETER`
   - Build arguments JSON object from parameters
   - Create `ToolCall` with `generate_call_id()`
   - Return `Vec<ToolCall>`

3. **E2-T3:** Implement `try_parse_xml_invoke()`
   - Helper function to parse a single `<invoke>` block
   - Returns `Option<ToolCall>`
   - Handles missing name gracefully (returns `None`)

4. **E2-T4:** Update `parse_tool_calls()` to try XML
   ```rust
   pub fn parse_tool_calls(content: &str) -> Vec<ToolCall> {
       let json_calls = parse_json_tool_calls(content);
       if !json_calls.is_empty() {
           tracing::debug!(
               num_calls = json_calls.len(),
               "Parsed tool calls from JSON format"
           );
           return json_calls;
       }

       let xml_calls = parse_xml_tool_calls(content);
       if !xml_calls.is_empty() {
           tracing::debug!(
               num_calls = xml_calls.len(),
               "Parsed tool calls from XML format"
           );
       }
       xml_calls
   }
   ```

**Acceptance Criteria:**
- XML tool calls correctly parsed
- JSON parsing still works (tested)
- Logs indicate which format was used
- Returns empty vec on malformed XML

---

### Epic 3: Add XML Stripping

**Goal:** Remove XML tool calls from content

**Tasks:**

1. **E3-T1:** Implement `strip_xml_tool_calls()`
   - Find `<function_calls>` blocks
   - Verify they contain valid tool calls (use `parse_xml_tool_calls()`)
   - Remove entire `<function_calls>...</function_calls>` block
   - Return cleaned string

2. **E3-T2:** Update `strip_tool_calls()` to strip XML
   ```rust
   pub fn strip_tool_calls(content: &str) -> String {
       let mut result = content.to_string();

       // Strip JSON (existing logic)
       // ... fenced blocks ...
       // ... inline JSON ...

       // Strip XML
       result = strip_xml_tool_calls(&result);

       // Collapse newlines and trim
       result = COLLAPSE_NEWLINES.replace_all(&result, "\n\n").to_string();
       result.trim().to_string()
   }
   ```

**Acceptance Criteria:**
- XML tool calls removed from content
- Surrounding text preserved
- Newlines collapsed properly
- JSON stripping still works

---

### Epic 4: Testing

**Goal:** Comprehensive test coverage

**File:** `tests/unit/tools_parser_tests.rs`

**Tasks:**

1. **E4-T1:** XML parsing tests - basic
   ```rust
   #[test]
   fn parse_single_xml_tool_call() {
       let content = r#"<function_calls>
   <invoke name="Bash">
   <parameter name="command">ls -la</parameter>
   </invoke>
   </function_calls>"#;

       let tool_calls = parse_tool_calls(content);
       assert_eq!(tool_calls.len(), 1);
       assert_eq!(tool_calls[0].function.name, Some("Bash".to_string()));

       let args: serde_json::Value =
           serde_json::from_str(tool_calls[0].function.arguments.as_ref().unwrap()).unwrap();
       assert_eq!(args["command"], "ls -la");
   }

   #[test]
   fn parse_xml_with_multiple_parameters() {
       let content = r#"<function_calls>
   <invoke name="Bash">
   <parameter name="command">git mv MISSING-FEATURES.md docs/</parameter>
   <parameter name="description">Move file</parameter>
   </invoke>
   </function_calls>"#;

       let tool_calls = parse_tool_calls(content);
       assert_eq!(tool_calls.len(), 1);

       let args: serde_json::Value =
           serde_json::from_str(tool_calls[0].function.arguments.as_ref().unwrap()).unwrap();
       assert_eq!(args["command"], "git mv MISSING-FEATURES.md docs/");
       assert_eq!(args["description"], "Move file");
   }

   #[test]
   fn parse_multiple_xml_invokes() {
       let content = r#"<function_calls>
   <invoke name="Bash">
   <parameter name="command">ls</parameter>
   </invoke>
   <invoke name="Grep">
   <parameter name="pattern">test</parameter>
   </invoke>
   </function_calls>"#;

       let tool_calls = parse_tool_calls(content);
       assert_eq!(tool_calls.len(), 2);
       assert_eq!(tool_calls[0].function.name, Some("Bash".to_string()));
       assert_eq!(tool_calls[1].function.name, Some("Grep".to_string()));
   }
   ```

2. **E4-T2:** XML parsing tests - edge cases
   ```rust
   #[test]
   fn parse_xml_with_surrounding_text() {
       let content = "Let me check that.\n\n<function_calls>...\n\nI'll run that now.";
       // Should parse tool calls and ignore surrounding text
   }

   #[test]
   fn parse_xml_malformed_missing_name() {
       let content = r#"<function_calls><invoke><parameter name="x">y</parameter></invoke></function_calls>"#;
       assert!(parse_tool_calls(content).is_empty());
   }

   #[test]
   fn parse_xml_empty_function_calls() {
       let content = "<function_calls></function_calls>";
       assert!(parse_tool_calls(content).is_empty());
   }

   #[test]
   fn parse_xml_no_parameters() {
       let content = r#"<function_calls><invoke name="NoOp"></invoke></function_calls>"#;
       let tool_calls = parse_tool_calls(content);
       assert_eq!(tool_calls.len(), 1);
       // Arguments should be empty object {}
   }
   ```

3. **E4-T3:** Mixed format tests
   ```rust
   #[test]
   fn parse_mixed_json_and_xml_prefers_json() {
       let content = r#"
   ```json
   {"function_call": {"name": "JsonTool", "arguments": {}}}
   ```

   <function_calls>
   <invoke name="XmlTool"><parameter name="x">y</parameter></invoke>
   </function_calls>
   "#;

       let tool_calls = parse_tool_calls(content);
       // Should parse JSON first
       assert!(!tool_calls.is_empty());
       assert_eq!(tool_calls[0].function.name, Some("JsonTool".to_string()));
   }
   ```

4. **E4-T4:** XML stripping tests
   ```rust
   #[test]
   fn strip_xml_tool_calls() {
       let content = "Before\n\n<function_calls>...\n\nAfter";
       let stripped = strip_tool_calls(content);
       assert!(!stripped.contains("<function_calls>"));
       assert!(stripped.contains("Before"));
       assert!(stripped.contains("After"));
   }

   #[test]
   fn strip_preserves_regular_xml() {
       let content = "See: <note>Important</note>";
       let stripped = strip_tool_calls(content);
       assert!(stripped.contains("<note>"));
   }
   ```

**Acceptance Criteria:**
- All new XML tests pass
- All existing JSON tests still pass
- Code coverage > 90% for new code
- Edge cases handled gracefully

---

### Epic 5: Documentation

**Tasks:**

1. **E5-T1:** Update inline code documentation
   - Document `parse_xml_tool_calls()` function
   - Document XML format in parser.rs comments
   - Update `parse_tool_calls()` docstring to mention both formats

2. **E5-T2:** Update debugging guide
   - Add XML format to expected formats in `docs/development/debugging-tool-calls.md`
   - Add example of XML in logs

**Acceptance Criteria:**
- Code is well-documented
- Debugging guide mentions both formats

---

## Critical Files

| File | Changes | Lines Changed (est.) |
|------|---------|---------------------|
| `src/tools/parser.rs` | Add XML parsing | ~200 lines |
| `tests/unit/tools_parser_tests.rs` | Add XML tests | ~150 lines |
| `docs/development/debugging-tool-calls.md` | Update examples | ~20 lines |

---

## Testing Strategy

### Unit Tests
```bash
cargo test --test unit tools_parser_tests
```

**Expected:**
- All existing JSON tests pass (no regressions)
- All new XML tests pass
- Mixed format tests pass

### Integration Testing

1. **Manual verification:**
   ```bash
   # Terminal 1
   copilot-adapter start --log-level trace

   # Terminal 2
   export ANTHROPIC_BASE_URL=http://127.0.0.1:6767
   export ANTHROPIC_API_KEY=dummy
   claude
   ```

2. **Make a request that triggers tools:**
   ```
   User: List the files in the current directory
   ```

3. **Check logs for:**
   ```
   TRACE Buffered content preview ...  # Shows XML or JSON
   DEBUG Parsed tool calls from XML format  # NEW - confirms XML parsed
   DEBUG Parsed tool calls from Anthropic response  # Confirms conversion
   ```

4. **Verify Claude Code:**
   - Executes the tool (runs `ls`)
   - Shows tool result
   - Does NOT display XML as text

### Regression Testing

Ensure no existing functionality breaks:
- JSON parsing still works
- Stripping still works
- Empty responses return empty vec
- Malformed input handled gracefully

---

## Verification

### Success Criteria

| Criterion | Verification Method |
|-----------|-------------------|
| XML tool calls parsed | Unit tests + manual testing with trace logs |
| JSON parsing unchanged | All existing tests pass |
| Claude Code executes tools | Manual testing - tools run, XML not displayed |
| No new dependencies | Check `Cargo.toml` unchanged |
| Performance acceptable | Parsing < 10ms (no noticeable latency) |

### Rollback Plan

If critical issues arise:
1. The change is isolated to `src/tools/parser.rs` and tests
2. Can revert the commit
3. No database or external state changes
4. No breaking API changes

---

## Timeline Estimate

| Epic | Est. Time | Dependencies |
|------|-----------|--------------|
| Epic 1: Extract JSON | 30 min | None |
| Epic 2: XML Parsing | 2 hours | Epic 1 |
| Epic 3: XML Stripping | 1 hour | Epic 2 |
| Epic 4: Testing | 2 hours | Epic 2, Epic 3 |
| Epic 5: Documentation | 30 min | Epic 2 |
| **Total** | **6 hours** | |

---

## Notes

- **Backwards compatible:** JSON parsing behavior unchanged
- **No new dependencies:** Uses existing `regex` crate from `Cargo.toml`
- **Graceful degradation:** If both parsers fail, returns empty (existing behavior)
- **Future-proof:** Can add more formats (e.g., YAML) following same pattern
- **Logging:** New debug logs help diagnose which format is used

---

## Related Documents

- [TOOLS-SUPPORT.plan.md](./TOOLS-SUPPORT.plan.md) - Original tool calling implementation
- [REGRESSION-FIX.md](./REGRESSION-FIX.md) - Related issue about missing tool definitions
- [debugging-tool-calls.md](../debugging-tool-calls.md) - Debugging guide (will be updated)
