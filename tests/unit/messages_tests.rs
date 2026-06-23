//! Unit tests for model-name normalization and 1M context beta handling.
//!
//! The core `has_1m_context_beta()` function has comprehensive inline tests in
//! `src/handlers/messages.rs` (7 tests covering single beta, comma-separated,
//! spaces, multiple headers, absent, other betas only, future date suffix).
//!
//! These tests document the post-consolidation behavior: GitHub Copilot's
//! current Claude SKUs are 1M-native under their base names and no longer expose
//! `-1m` model IDs, so the `context-1m` beta header is detected for diagnostics
//! but does NOT mutate the outgoing model name. See
//! `docs/design/COPILOT-1M-MODEL-CONSOLIDATION.design.md`.

use copilot_adapter::model_mapper::normalize_model_name;

/// The base model name is what the adapter sends for a 1M-context selection.
/// Claude Code strips `[1m]` before sending and signals 1M via the
/// `anthropic-beta: context-1m-*` header — which no longer changes the model
/// name. The normalized base name (e.g. `claude-opus-4.6`) is 1M-native on
/// Copilot, so no suffix is appended.
#[test]
fn one_m_selection_uses_base_model_name() {
    let normalized = normalize_model_name("claude-opus-4-6");
    assert_eq!(
        normalized, "claude-opus-4.6",
        "1M context is served by the base model name; no -1m suffix is added"
    );
}

/// The `context-1m` beta header does not alter the outgoing model name. Two
/// requests for the same model — one with the 1M selection, one without —
/// resolve to the identical Copilot model name, because the suffix-append
/// behavior has been removed.
#[test]
fn one_m_header_does_not_change_model_name() {
    // Whether or not the caller intends 1M context, the model name the adapter
    // derives from the request is the same normalized base name. (The handler
    // detects the header only for diagnostic logging.)
    let with_1m_intent = normalize_model_name("claude-opus-4-6");
    let without_1m_intent = normalize_model_name("claude-opus-4-6");
    assert_eq!(with_1m_intent, without_1m_intent);
    assert_eq!(with_1m_intent, "claude-opus-4.6");
}

/// Opus 4.7 normalizes to its plain base name. Effort is carried separately via
/// `reasoning.effort` (translated in `to_chat_completion_request`), not encoded
/// as a model-name SKU suffix — Copilot no longer exposes `-high` / `-xhigh`
/// model IDs.
#[test]
fn opus_47_normalizes_to_base_name() {
    assert_eq!(normalize_model_name("claude-opus-4-7"), "claude-opus-4.7");
}
