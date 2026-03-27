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
// Public API
// ---------------------------------------------------------------------------

/// Parse tool calls from model-generated text content.
///
/// Looks for JSON blocks matching the injected tool-call format:
/// ```json
/// {"function_call": {"name": "func_name", "arguments": {...}}}
/// ```
///
/// The function first tries fenced code blocks (```json ... ```), then falls
/// back to inline JSON objects containing `"function_call"`.
///
/// Each extracted tool call is assigned a unique `call_xxx` ID.
/// Multiple tool calls are returned in document order.
///
/// Invalid or malformed JSON is silently skipped.
pub fn parse_tool_calls(content: &str) -> Vec<ToolCall> {
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

/// Strip tool call text from the content, returning the cleaned text.
///
/// Removes both fenced code blocks and inline JSON tool calls, leaving
/// surrounding prose intact. Only fenced blocks that parse as valid tool
/// calls are removed — regular JSON documentation examples are preserved.
/// Extra blank lines left by removal are collapsed.
///
/// **Note:** The result is always trimmed of leading/trailing whitespace
/// regardless of whether any tool calls were removed.
pub fn strip_tool_calls(content: &str) -> String {
    let mut result = content.to_string();

    // Remove fenced code blocks, but only those containing valid tool calls.
    // We iterate over captures, check each for a valid tool call, and remove
    // matches in reverse order to preserve byte offsets.
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

    // Remove inline JSON tool calls using brace-counting.
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
