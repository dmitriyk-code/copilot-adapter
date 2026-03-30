//! Tool call parser for model-generated text content.
//!
//! This module extracts tool calls from assistant responses. It supports two
//! formats (tried in order):
//!
//! 1. **JSON** (legacy) — `{"function_call": {"name": "...", "arguments": {...}}}`
//!    inside fenced `` ```json `` code blocks or inline.
//! 2. **XML** (primary) —
//!    ```xml
//!    <function_calls>
//!      <invoke name="ToolName">
//!        <parameter name="key">value</parameter>
//!      </invoke>
//!    </function_calls>
//!    ```
//!
//! XML is the format requested via prompt injection (see
//! `src/tools/injector.rs`). JSON is retained as a legacy fallback. If both
//! formats appear in the same response, JSON takes priority.
//!
//! The public API consists of two functions:
//! - [`parse_tool_calls`] — extract `ToolCall` structs from text
//! - [`strip_tool_calls`] — remove tool call markup, leaving surrounding prose

use once_cell::sync::Lazy;
use regex::Regex;
use uuid::Uuid;

use crate::tools::types::{FunctionCall, ToolCall};

// ---------------------------------------------------------------------------
// Regex patterns — compiled once
// ---------------------------------------------------------------------------

/// Matches a tool call inside a fenced ```json code block.
///
/// Captures the JSON content between the fences. The pattern allows optional
/// whitespace around the JSON and handles multi-line content. The `(?s)` flag
/// makes `.` match newlines.
static FENCED_PATTERN: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"(?s)```json\s*\n(\{.*?\})\s*\n```").expect("fenced regex should compile")
});

/// Finds the start of a potential inline tool call JSON object.
/// Used to locate candidates for brace-counting extraction.
static INLINE_START: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r#"\{"function_call"\s*:"#).expect("inline start regex should compile")
});

/// Collapses runs of 3+ newlines down to 2 (one blank line).
static COLLAPSE_NEWLINES: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"\n{3,}").expect("collapse regex should compile")
});

// ---------------------------------------------------------------------------
// XML regex patterns — compiled once
// ---------------------------------------------------------------------------

/// Matches `<function_calls>...</function_calls>` blocks.
///
/// The `(?s)` flag makes `.` match newlines so multi-line XML is captured.
static XML_FUNCTION_CALLS: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"(?s)<function_calls>(.*?)</function_calls>")
        .expect("xml function_calls regex should compile")
});

/// Matches `<invoke name="...">...</invoke>` blocks within a function_calls body.
///
/// The name capture uses `[^"]*` (zero-or-more) so that empty names are captured
/// and rejected by the `try_parse_xml_invoke` guard rather than silently ignored.
static XML_INVOKE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r#"(?s)<invoke\s+name="([^"]*)">(.*?)</invoke>"#)
        .expect("xml invoke regex should compile")
});

/// Matches `<parameter name="...">...</parameter>` within an invoke body.
///
/// Parameter values cannot contain `<` (no nested XML).
static XML_PARAMETER: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r#"<parameter\s+name="([^"]+)">([^<]*)</parameter>"#)
        .expect("xml parameter regex should compile")
});

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Parse tool calls from model-generated text content.
///
/// Supports two formats:
/// 1. **JSON** (primary): `{"function_call": {"name": "...", "arguments": {...}}}`
///    in fenced code blocks or inline.
/// 2. **XML** (fallback): `<function_calls><invoke name="...">...</invoke></function_calls>`
///
/// JSON is tried first. If no JSON tool calls are found, the function falls
/// back to XML parsing. This ensures backwards compatibility while handling
/// Claude models that generate their native XML format.
///
/// Each extracted tool call is assigned a unique `call_xxx` ID.
/// Multiple tool calls are returned in document order.
///
/// Invalid or malformed content is silently skipped.
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

// ---------------------------------------------------------------------------
// Format-specific parsers
// ---------------------------------------------------------------------------

/// Parse JSON-format tool calls from model-generated text content.
///
/// Looks for JSON blocks matching the injected tool-call format:
/// ```json
/// {"function_call": {"name": "func_name", "arguments": {...}}}
/// ```
///
/// Tries fenced code blocks (` ```json ... ``` `) first, then falls back to
/// inline JSON objects containing `"function_call"`.
fn parse_json_tool_calls(content: &str) -> Vec<ToolCall> {
    let mut candidates: Vec<(usize, usize, String)> = Vec::new(); // (start, end, json_str)

    // Collect fenced code block matches
    for cap in FENCED_PATTERN.captures_iter(content) {
        let full_match = cap.get(0).unwrap();
        let json_str = cap.get(1).unwrap().as_str();
        candidates.push((full_match.start(), full_match.end(), json_str.to_string()));
    }

    // Collect inline JSON matches using brace-counting
    for mat in INLINE_START.find_iter(content) {
        let start = mat.start();
        // Skip if this start position is inside a fenced block
        if candidates.iter().any(|(cs, ce, _)| start >= *cs && start < *ce) {
            continue;
        }
        if let Some(end) = find_matching_brace(content, start) {
            let json_str = &content[start..end];
            candidates.push((start, end, json_str.to_string()));
        }
    }

    // Sort by position for document-order output
    candidates.sort_by_key(|(start, _, _)| *start);

    // Deduplicate overlapping candidates (fenced takes priority)
    let mut used_ranges: Vec<(usize, usize)> = Vec::new();
    let mut tool_calls = Vec::new();

    for (start, end, json_str) in &candidates {
        if is_overlapping(&used_ranges, *start, *end) {
            continue;
        }
        if let Some(tc) = try_parse_tool_call(json_str) {
            used_ranges.push((*start, *end));
            tool_calls.push(tc);
        }
    }

    tool_calls
}

/// Parse XML-format tool calls from model-generated text content.
///
/// Looks for `<function_calls>` blocks containing `<invoke>` elements:
/// ```xml
/// <function_calls>
///   <invoke name="ToolName">
///     <parameter name="param1">value1</parameter>
///     <parameter name="param2">value2</parameter>
///   </invoke>
/// </function_calls>
/// ```
///
/// Claude models sometimes generate this format instead of the JSON format
/// requested via prompt injection. This parser handles it as a fallback.
fn parse_xml_tool_calls(content: &str) -> Vec<ToolCall> {
    let mut tool_calls = Vec::new();

    for fc_cap in XML_FUNCTION_CALLS.captures_iter(content) {
        let body = fc_cap.get(1).unwrap().as_str();

        for invoke_cap in XML_INVOKE.captures_iter(body) {
            let name = invoke_cap.get(1).unwrap().as_str();
            let invoke_body = invoke_cap.get(2).unwrap().as_str();

            if let Some(tc) = try_parse_xml_invoke(name, invoke_body) {
                tool_calls.push(tc);
            }
        }
    }

    tool_calls
}

/// Parse a single XML `<invoke>` block into a `ToolCall`.
///
/// `name` is the tool name from the `name` attribute.
/// `invoke_body` is the inner content of the `<invoke>` element (parameters).
///
/// Returns `None` if the name is empty.
fn try_parse_xml_invoke(name: &str, invoke_body: &str) -> Option<ToolCall> {
    if name.is_empty() {
        return None;
    }

    let mut params = serde_json::Map::new();

    for param_cap in XML_PARAMETER.captures_iter(invoke_body) {
        let param_name = param_cap.get(1).unwrap().as_str();
        let param_value = param_cap.get(2).unwrap().as_str().trim();
        params.insert(
            param_name.to_string(),
            serde_json::Value::String(param_value.to_string()),
        );
    }

    let arguments = if params.is_empty() {
        Some("{}".to_string())
    } else {
        Some(serde_json::Value::Object(params).to_string())
    };

    Some(ToolCall {
        id: Some(generate_call_id()),
        call_type: Some("function".to_string()),
        function: FunctionCall {
            name: Some(name.to_string()),
            arguments,
        },
    })
}

/// Strip tool call text from the content, returning the cleaned text.
///
/// Removes JSON tool calls (fenced code blocks + inline) and XML
/// `<function_calls>` blocks, leaving surrounding prose intact. Only blocks
/// that parse as valid tool calls are removed — regular code examples are
/// preserved. Extra blank lines left by removal are collapsed.
///
/// **Note:** The result is always trimmed of leading/trailing whitespace
/// regardless of whether any tool calls were removed.
pub fn strip_tool_calls(content: &str) -> String {
    let mut result = content.to_string();

    // ----- JSON: fenced code blocks -----
    // Only remove fenced blocks that parse as valid tool calls.
    let fenced_removals: Vec<(usize, usize)> = FENCED_PATTERN
        .captures_iter(&result)
        .filter_map(|cap| {
            let full_match = cap.get(0)?;
            let json_str = cap.get(1)?.as_str();
            if try_parse_tool_call(json_str).is_some() {
                Some((full_match.start(), full_match.end()))
            } else {
                None
            }
        })
        .collect();

    // Remove in reverse order so earlier offsets remain valid
    for (start, end) in fenced_removals.into_iter().rev() {
        result.replace_range(start..end, "");
    }

    // ----- JSON: inline objects -----
    // Track a search offset so we skip past non-valid matches instead of
    // stopping at the first failure.
    let mut search_from = 0;
    loop {
        let haystack = &result[search_from..];
        let mat = INLINE_START.find(haystack);
        match mat {
            Some(m) => {
                let abs_start = search_from + m.start();
                if let Some(abs_end) = find_matching_brace(&result, abs_start) {
                    let json_str = &result[abs_start..abs_end];
                    if try_parse_tool_call(json_str).is_some() {
                        result.replace_range(abs_start..abs_end, "");
                        // Don't advance search_from — positions shifted, rescan
                        // from same offset.
                        continue;
                    }
                    // Not a valid tool call — advance past this match's start
                    search_from = abs_start + 1;
                } else {
                    // Unbalanced braces — skip past this candidate
                    search_from = abs_start + 1;
                }
            }
            None => break,
        }
    }

    // ----- XML: <function_calls> blocks -----
    // Only remove blocks that contain at least one valid <invoke>.
    let xml_removals: Vec<(usize, usize)> = XML_FUNCTION_CALLS
        .captures_iter(&result)
        .filter_map(|cap| {
            let full_match = cap.get(0)?;
            let body = cap.get(1)?.as_str();
            // Check that at least one invoke parses successfully
            let has_valid_invoke = XML_INVOKE.captures_iter(body).any(|invoke_cap| {
                let name = invoke_cap.get(1).unwrap().as_str();
                let invoke_body = invoke_cap.get(2).unwrap().as_str();
                try_parse_xml_invoke(name, invoke_body).is_some()
            });
            if has_valid_invoke {
                Some((full_match.start(), full_match.end()))
            } else {
                None
            }
        })
        .collect();

    for (start, end) in xml_removals.into_iter().rev() {
        result.replace_range(start..end, "");
    }

    // Collapse runs of 3+ newlines down to 2 (one blank line)
    result = COLLAPSE_NEWLINES.replace_all(&result, "\n\n").to_string();

    result.trim().to_string()
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

/// Find the position just past the matching closing brace for the `{` at `start`.
///
/// Handles nested braces and respects JSON string escaping (skips braces
/// inside quoted strings).
fn find_matching_brace(content: &str, start: usize) -> Option<usize> {
    let bytes = content.as_bytes();
    if start >= bytes.len() || bytes[start] != b'{' {
        return None;
    }

    let mut depth = 0;
    let mut in_string = false;
    let mut escape_next = false;
    let mut i = start;

    while i < bytes.len() {
        let ch = bytes[i];

        if escape_next {
            escape_next = false;
            i += 1;
            continue;
        }

        if ch == b'\\' && in_string {
            escape_next = true;
            i += 1;
            continue;
        }

        if ch == b'"' {
            in_string = !in_string;
        } else if !in_string {
            if ch == b'{' {
                depth += 1;
            } else if ch == b'}' {
                depth -= 1;
                if depth == 0 {
                    return Some(i + 1);
                }
            }
        }

        i += 1;
    }

    None // unbalanced braces
}

/// Try to parse a JSON string as a tool call.
///
/// Expects the format: `{"function_call": {"name": "...", "arguments": {...}}}`
/// Returns `None` if the JSON is malformed or doesn't match the expected schema.
fn try_parse_tool_call(json_str: &str) -> Option<ToolCall> {
    let value: serde_json::Value = serde_json::from_str(json_str).ok()?;
    let fc = value.get("function_call")?;

    let name = fc.get("name")?.as_str()?.to_string();

    // Arguments can be any JSON value — we store it as a raw JSON string
    let arguments = fc.get("arguments").map(|v| v.to_string());

    let id = generate_call_id();

    Some(ToolCall {
        id: Some(id),
        call_type: Some("function".to_string()),
        function: FunctionCall {
            name: Some(name),
            arguments,
        },
    })
}

/// Generate a unique tool call ID in the format `call_<short-hex>`.
fn generate_call_id() -> String {
    let uuid = Uuid::new_v4();
    // Use first 12 hex chars of the UUID for a compact but unique ID
    let hex = uuid.as_simple().to_string();
    format!("call_{}", &hex[..12])
}

/// Check if a range overlaps with any existing range.
fn is_overlapping(ranges: &[(usize, usize)], start: usize, end: usize) -> bool {
    ranges.iter().any(|(rs, re)| start < *re && end > *rs)
}

// ---------------------------------------------------------------------------
// Unit tests (inline)
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generate_call_id_has_correct_prefix() {
        let id = generate_call_id();
        assert!(id.starts_with("call_"));
        assert_eq!(id.len(), 17); // "call_" (5) + 12 hex chars
    }

    #[test]
    fn generate_call_id_is_unique() {
        let id1 = generate_call_id();
        let id2 = generate_call_id();
        assert_ne!(id1, id2);
    }

    #[test]
    fn try_parse_valid_tool_call() {
        let json = r#"{"function_call": {"name": "bash", "arguments": {"command": "ls"}}}"#;
        let tc = try_parse_tool_call(json).unwrap();
        assert_eq!(tc.function.name, Some("bash".to_string()));
        assert!(tc.function.arguments.is_some());
        assert!(tc.id.unwrap().starts_with("call_"));
        assert_eq!(tc.call_type, Some("function".to_string()));
    }

    #[test]
    fn try_parse_missing_function_call_key() {
        let json = r#"{"name": "bash", "arguments": {}}"#;
        assert!(try_parse_tool_call(json).is_none());
    }

    #[test]
    fn try_parse_missing_name() {
        let json = r#"{"function_call": {"arguments": {}}}"#;
        assert!(try_parse_tool_call(json).is_none());
    }

    #[test]
    fn try_parse_invalid_json() {
        let json = r#"{not valid json"#;
        assert!(try_parse_tool_call(json).is_none());
    }

    #[test]
    fn find_matching_brace_simple() {
        let s = r#"{"key": "value"}"#;
        assert_eq!(find_matching_brace(s, 0), Some(16));
    }

    #[test]
    fn find_matching_brace_nested() {
        let s = r#"{"a": {"b": {"c": 1}}}"#;
        assert_eq!(find_matching_brace(s, 0), Some(22));
    }

    #[test]
    fn find_matching_brace_with_escaped_quotes() {
        let s = r#"{"key": "val\"ue"}"#;
        assert_eq!(find_matching_brace(s, 0), Some(18));
    }

    #[test]
    fn find_matching_brace_with_brace_in_string() {
        let s = r#"{"key": "}{{"}"#;
        assert_eq!(find_matching_brace(s, 0), Some(14));
    }

    #[test]
    fn find_matching_brace_unbalanced() {
        let s = r#"{"key": "value""#;
        assert!(find_matching_brace(s, 0).is_none());
    }

    #[test]
    fn is_overlapping_detects_overlap() {
        let ranges = vec![(10, 50)];
        assert!(is_overlapping(&ranges, 20, 30));
        assert!(is_overlapping(&ranges, 0, 15));
        assert!(is_overlapping(&ranges, 45, 60));
        assert!(!is_overlapping(&ranges, 50, 60));
        assert!(!is_overlapping(&ranges, 0, 10));
    }
}
