/// Normalize Claude model names from Claude Code's versioned format to GitHub
/// Copilot's expected format.
///
/// Claude Code uses versioned model identifiers like:
/// - `claude-haiku-4-5-20251001` (with datestamp)
/// - `claude-sonnet-4-5-20251022`
/// - `claude-opus-4-6-1m-20251120` (with context size marker)
///
/// GitHub Copilot expects simple model names like:
/// - `claude-haiku-4.5`
/// - `claude-sonnet-4.5`
/// - `claude-opus-4.6`
///
/// This function normalizes the model names by:
/// 1. Stripping datestamp suffixes (e.g., `-20251001`)
/// 2. Removing context size markers (e.g., `-1m`)
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
        // Split by dashes
        let parts: Vec<&str> = model.split('-').collect();

        // Expected format: claude-{family}-{major}-{minor}[-{context}][-{datestamp}]
        // Examples:
        // - claude-haiku-4-5-20251001 → claude-haiku-4.5
        // - claude-opus-4-6-1m-20251120 → claude-opus-4.6
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

        // Construct normalized model name
        format!("claude-{}-{}.{}", family, major, minor)
    } else {
        // Unknown format, return as-is
        model.to_string()
    }
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
            "claude-opus-4.6"
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
}
