//! Profile data types and validation.

use anyhow::Result;
use std::path::PathBuf;

/// A named profile with an associated directory for status and credentials.
#[derive(Debug, Clone)]
pub struct Profile {
    pub name: String,
    pub dir: PathBuf,
}

impl Profile {
    /// Path to this profile's runtime status file.
    pub fn status_path(&self) -> PathBuf {
        self.dir.join("status.json")
    }

    /// Path to this profile's credential storage file.
    ///
    /// Returns the path to `github-copilot.json`, which uses platform-native
    /// encryption (DPAPI on Windows, keyring on macOS/Linux).
    pub fn credentials_path(&self) -> PathBuf {
        self.dir.join("github-copilot.json")
    }
}

/// Validate a profile name.
///
/// Rules:
/// - 1–64 characters
/// - Only ASCII alphanumeric, dash (`-`), and underscore (`_`)
pub fn validate_profile_name(name: &str) -> Result<()> {
    if name.is_empty() || name.len() > 64 {
        anyhow::bail!("Profile name must be 1-64 characters");
    }
    if !name
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        anyhow::bail!("Profile name may only contain letters, digits, dash, underscore");
    }
    Ok(())
}
