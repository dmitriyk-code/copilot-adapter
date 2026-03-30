# Model Name Normalization

## Problem

Claude Code uses **versioned model identifiers** with datestamps that don't match the model names GitHub Copilot expects:

**Claude Code sends:**
- `claude-haiku-4-5-20251001` (with date suffix)
- `claude-sonnet-4-5-20251022` (with date suffix)
- `claude-opus-4-6-1m-20251120` (with context size marker + date)

**GitHub Copilot expects:**
- `claude-haiku-4.5` (simple name, dots instead of dashes)
- `claude-sonnet-4.5`
- `claude-opus-4.6`

When the adapter passed these versioned names directly to Copilot, the API either rejected them or used a fallback model.

## Solution

Added automatic model name normalization in `src/model_mapper.rs` that:

1. **Strips datestamp suffixes** (e.g., `-20251001`)
2. **Removes context size markers** (e.g., `-1m`)
3. **Converts dashes to dots** in version numbers (e.g., `4-5` → `4.5`)

The normalization is applied at the `/v1/messages` endpoint (Anthropic format) in `src/anthropic/types.rs`.

## Implementation

```rust
pub fn normalize_model_name(model: &str) -> String {
    // Handle Claude models with versioned format
    if model.starts_with("claude-") {
        // claude-haiku-4-5-20251001 → claude-haiku-4.5
        // claude-opus-4-6-1m-20251120 → claude-opus-4.6

        let parts: Vec<&str> = model.split('-').collect();
        let family = parts[1]; // haiku, sonnet, opus
        let major = parts[2];  // 4
        let minor = parts[3];  // 5

        format!("claude-{}-{}.{}", family, major, minor)
    } else {
        // GPT and other models pass through unchanged
        model.to_string()
    }
}
```

## Logging

When model normalization occurs, the adapter logs it at `INFO` level:

```
Model name normalized for GitHub Copilot compatibility
  original_model: claude-haiku-4-5-20251001
  normalized_model: claude-haiku-4.5
```

This helps verify the adapter is correctly transforming model names.

## Testing

Unit tests in `src/model_mapper.rs` verify the normalization logic:

```bash
cargo test model_mapper
```

Tests cover:
- Claude models with datestamps
- Claude models with context size markers + dates
- Claude models already normalized
- GPT models (pass-through)
- Gemini models (pass-through)

## Why `/v1/models` Isn't Called

Claude Code **does not** call the `/v1/models` endpoint when you type `/model`. Instead, it uses a **static configuration** in its settings file:

```json
{
  "env": {
    "ANTHROPIC_MODEL": "claude-sonnet-4.5",
    "ANTHROPIC_DEFAULT_OPUS_MODEL": "claude-opus-4.5",
    "ANTHROPIC_SMALL_FAST_MODEL": "claude-haiku-4.5"
  }
}
```

The `/v1/models` endpoint is primarily for:
- Other OpenAI-compatible clients that discover models dynamically
- Initial setup/validation when configuring the adapter
- API compatibility testing

Claude Code itself has a built-in model list and adds the version suffixes internally before sending requests.
