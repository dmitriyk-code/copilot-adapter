/// Normalize Claude model names from Claude Code's versioned format to GitHub
/// Copilot's expected format.
///
/// Claude Code uses versioned model identifiers like:
/// - `claude-haiku-4-5-20251001` (with datestamp)
/// - `claude-sonnet-4-5-20251022`
/// - `claude-opus-4-6-1m-20251120` (with context size marker + datestamp)
///
/// GitHub Copilot expects model names like:
/// - `claude-haiku-4.5`
/// - `claude-sonnet-4.5`
/// - `claude-opus-4.6-1m` (preserves context marker)
///
/// This function normalizes the model names by:
/// 1. Stripping datestamp suffixes (e.g., `-20251001`)
/// 2. Preserving context size markers (e.g., `-1m`)
/// 3. Converting dashes to dots in version numbers (e.g., `4-5` → `4.5`)
pub fn normalize_model_name(model: &str) -> String {
    // If already in correct format, return as-is
    if model.starts_with("gpt-")
        || model.starts_with("gemini-")
        || model.starts_with("text-embedding-")
    {
        return model.to_string();
    }

    // Handle Claude models with versioned format
    if model.starts_with("claude-") {
        // Check if already normalized (contains dots in version)
        if model.contains('.') {
            // Already normalized, return as-is
            return model.to_string();
        }

        // Split by dashes
        let parts: Vec<&str> = model.split('-').collect();

        // Expected format: claude-{family}-{major}-{minor}[-{context}][-{datestamp}]
        // Examples:
        // - claude-haiku-4-5-20251001 → claude-haiku-4.5
        // - claude-opus-4-6-1m-20251120 → claude-opus-4.6-1m
        // - claude-sonnet-4-5 → claude-sonnet-4.5 (already normalized, no date)

        if parts.len() < 4 {
            // Not in expected format, return as-is
            return model.to_string();
        }

        let family = parts[1]; // haiku, sonnet, opus
        let major = parts[2]; // 4
        let minor = parts[3]; // 5

        // Guard: if parts[3] is a datestamp (8-digit number like 20250514),
        // not a minor version, return the model name as-is.
        if minor.len() == 8 && minor.chars().all(|c| c.is_ascii_digit()) {
            return model.to_string();
        }

        // Check for context marker and datestamp in remaining parts
        let mut context_marker = None;

        if parts.len() > 4 {
            // Look for context marker (e.g., "1m", "200k") - non-datestamp numeric suffixes
            for part in &parts[4..] {
                if !part.is_empty()
                    && !is_datestamp(part)
                    && (part.ends_with('m') || part.ends_with('k') || part.chars().all(|c| c.is_ascii_digit()))
                {
                    context_marker = Some(*part);
                    break;
                }
            }
        }

        // Construct normalized model name
        if let Some(marker) = context_marker {
            format!("claude-{}-{}.{}-{}", family, major, minor, marker)
        } else {
            format!("claude-{}-{}.{}", family, major, minor)
        }
    } else {
        // Unknown format, return as-is
        model.to_string()
    }
}

/// Apply effort and 1M-context modifiers to an already-normalized model name.
///
/// GitHub Copilot's opus 4.7 series encodes effort level and context size as
/// **model name suffixes** rather than accepting them as separate API fields:
/// - `claude-opus-4.7`           — standard (no modifiers)
/// - `claude-opus-4.7-high`      — high effort
/// - `claude-opus-4.7-xhigh`     — xhigh effort
/// - `claude-opus-4.7-1m-internal` — 1M context (no combined effort+1M variant)
///
/// Earlier models (e.g. opus 4.6) use the older convention: `reasoning.effort`
/// is sent as a separate request field, and 1M context appends `-1m`.
///
/// Returns `(final_model_name, suppress_reasoning)`.
/// `suppress_reasoning` is `true` when effort is encoded in the model name —
/// in that case the caller must clear the `reasoning` field from the request
/// body to avoid sending a redundant (and potentially rejected) parameter.
pub fn apply_model_modifiers(
    model: &str,
    effort: Option<&str>,
    wants_1m: bool,
) -> (String, bool) {
    // opus 4.7 — effort and 1M context are encoded as model name suffixes.
    if model == "claude-opus-4.7" {
        if wants_1m {
            // The 1M-context model is a single fixed variant; no effort combinations
            // are available for 1M context in the current Copilot model catalogue.
            return ("claude-opus-4.7-1m-internal".to_string(), true);
        }
        match effort {
            Some("xhigh") => return ("claude-opus-4.7-xhigh".to_string(), true),
            // "max" maps to "high" for consistency with the older effort mapping
            Some("high") | Some("max") => return ("claude-opus-4.7-high".to_string(), true),
            _ => return (model.to_string(), false),
        }
    }

    // All other models: append `-1m` when 1M context is requested, and pass
    // effort separately via `reasoning.effort` (handled by the request builder).
    let final_model = if wants_1m && !model.contains("-1m") {
        format!("{}-1m", model)
    } else {
        model.to_string()
    };

    (final_model, false)
}

/// Helper function to check if a string looks like a datestamp (8 digits)
fn is_datestamp(s: &str) -> bool {
    s.len() == 8 && s.chars().all(|c| c.is_ascii_digit())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_normalize_claude_haiku_with_date() {
        assert_eq!(
            normalize_model_name("claude-haiku-4-5-20251001"),
            "claude-haiku-4.5"
        );
    }

    #[test]
    fn test_normalize_claude_opus_with_context_and_date() {
        assert_eq!(
            normalize_model_name("claude-opus-4-6-1m-20251120"),
            "claude-opus-4.6-1m"
        );
    }

    #[test]
    fn test_normalize_claude_sonnet_already_normalized() {
        assert_eq!(
            normalize_model_name("claude-sonnet-4-5"),
            "claude-sonnet-4.5"
        );
    }

    #[test]
    fn test_normalize_claude_sonnet_with_dots() {
        // Already has dots, but we'll process it anyway
        assert_eq!(
            normalize_model_name("claude-sonnet-4.5"),
            "claude-sonnet-4.5"
        );
    }

    #[test]
    fn test_normalize_gpt_model() {
        assert_eq!(normalize_model_name("gpt-4o"), "gpt-4o");
    }

    #[test]
    fn test_normalize_gemini_model() {
        assert_eq!(
            normalize_model_name("gemini-3-flash-preview"),
            "gemini-3-flash-preview"
        );
    }

    #[test]
    fn test_normalize_claude_with_datestamp_only() {
        // claude-sonnet-4-20250514 has parts[3] = "20250514" (a datestamp, not minor version)
        assert_eq!(
            normalize_model_name("claude-sonnet-4-20250514"),
            "claude-sonnet-4-20250514"
        );
    }

    #[test]
    fn test_normalize_claude_opus_with_datestamp_only() {
        assert_eq!(
            normalize_model_name("claude-opus-4-20250514"),
            "claude-opus-4-20250514"
        );
    }

    #[test]
    fn test_normalize_claude_with_context_marker_only() {
        // claude-opus-4-6-1m → claude-opus-4.6-1m
        assert_eq!(
            normalize_model_name("claude-opus-4-6-1m"),
            "claude-opus-4.6-1m"
        );
    }

    #[test]
    fn test_normalize_claude_with_200k_context() {
        // claude-sonnet-4-5-200k → claude-sonnet-4.5-200k
        assert_eq!(
            normalize_model_name("claude-sonnet-4-5-200k"),
            "claude-sonnet-4.5-200k"
        );
    }

    #[test]
    fn test_normalize_claude_with_context_and_multiple_parts() {
        // claude-haiku-4-5-200k-20251001 → claude-haiku-4.5-200k
        assert_eq!(
            normalize_model_name("claude-haiku-4-5-200k-20251001"),
            "claude-haiku-4.5-200k"
        );
    }

    #[test]
    fn test_normalize_claude_already_normalized_with_context() {
        // Already normalized with context marker
        assert_eq!(
            normalize_model_name("claude-opus-4.6-1m"),
            "claude-opus-4.6-1m"
        );
    }

    // --- apply_model_modifiers tests ---

    #[test]
    fn test_opus47_xhigh_effort() {
        let (model, suppress) = apply_model_modifiers("claude-opus-4.7", Some("xhigh"), false);
        assert_eq!(model, "claude-opus-4.7-xhigh");
        assert!(suppress);
    }

    #[test]
    fn test_opus47_high_effort() {
        let (model, suppress) = apply_model_modifiers("claude-opus-4.7", Some("high"), false);
        assert_eq!(model, "claude-opus-4.7-high");
        assert!(suppress);
    }

    #[test]
    fn test_opus47_max_effort_maps_to_high() {
        let (model, suppress) = apply_model_modifiers("claude-opus-4.7", Some("max"), false);
        assert_eq!(model, "claude-opus-4.7-high");
        assert!(suppress);
    }

    #[test]
    fn test_opus47_no_effort() {
        let (model, suppress) = apply_model_modifiers("claude-opus-4.7", None, false);
        assert_eq!(model, "claude-opus-4.7");
        assert!(!suppress);
    }

    #[test]
    fn test_opus47_1m_context() {
        let (model, suppress) = apply_model_modifiers("claude-opus-4.7", None, true);
        assert_eq!(model, "claude-opus-4.7-1m-internal");
        assert!(suppress);
    }

    #[test]
    fn test_opus47_1m_context_with_effort_ignored() {
        // 1M context takes priority; no combined effort+1M variant available
        let (model, suppress) = apply_model_modifiers("claude-opus-4.7", Some("xhigh"), true);
        assert_eq!(model, "claude-opus-4.7-1m-internal");
        assert!(suppress);
    }

    #[test]
    fn test_opus46_1m_context() {
        let (model, suppress) = apply_model_modifiers("claude-opus-4.6", None, true);
        assert_eq!(model, "claude-opus-4.6-1m");
        assert!(!suppress);
    }

    #[test]
    fn test_opus46_no_modifiers() {
        let (model, suppress) = apply_model_modifiers("claude-opus-4.6", None, false);
        assert_eq!(model, "claude-opus-4.6");
        assert!(!suppress);
    }

    #[test]
    fn test_non_claude_model_no_change() {
        let (model, suppress) = apply_model_modifiers("gpt-4o", None, false);
        assert_eq!(model, "gpt-4o");
        assert!(!suppress);
    }
}
