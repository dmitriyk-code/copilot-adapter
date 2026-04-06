//! Epic 5 Task 5.3: Unit tests for 1M context beta detection
//!
//! The core `has_1m_context_beta()` function has comprehensive inline tests in
//! `src/handlers/messages.rs` (7 tests covering single beta, comma-separated,
//! spaces, multiple headers, absent, other betas only, future date suffix).
//!
//! This file adds tests for the model name normalization + 1M suffix
//! interaction and the no-double-append guard which requires integration-level
//! testing (see integration/messages_tests.rs for the full end-to-end version).

use copilot_adapter::model_mapper::normalize_model_name;

#[test]
fn model_name_with_1m_beta_appends_suffix() {
    let normalized = normalize_model_name("claude-opus-4-6");
    assert_eq!(normalized, "claude-opus-4.6");
    let with_1m = format!("{}-1m", normalized);
    assert_eq!(with_1m, "claude-opus-4.6-1m");
}

/// Verify the no-double-append guard using the same logic as the production
/// handler: `!model.contains("-1m")`. When a model name already contains "-1m"
/// (e.g. `claude-opus-4.6-1m` after normalization of `claude-opus-4-6-1m`),
/// the guard prevents a second `-1m` from being appended.
#[test]
fn model_name_no_double_append_guard() {
    // Simulate the production flow: normalize → check guard → conditionally append
    let normalized = normalize_model_name("claude-opus-4-6-1m");
    assert_eq!(normalized, "claude-opus-4.6-1m");

    // Production guard: only append if "-1m" not already present
    let wants_1m = true;
    let result = if wants_1m && !normalized.contains("-1m") {
        format!("{}-1m", normalized)
    } else {
        normalized.clone()
    };
    assert_eq!(result, "claude-opus-4.6-1m", "Guard must prevent double -1m suffix");

    // Also verify the guard DOES append when -1m is absent
    let without_1m = normalize_model_name("claude-opus-4-6");
    assert_eq!(without_1m, "claude-opus-4.6");
    let result2 = if wants_1m && !without_1m.contains("-1m") {
        format!("{}-1m", without_1m)
    } else {
        without_1m.clone()
    };
    assert_eq!(result2, "claude-opus-4.6-1m", "Guard must append -1m when absent");
}
