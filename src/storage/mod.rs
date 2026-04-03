pub mod legacy;
pub mod native;

#[cfg(target_os = "windows")]
pub mod dpapi;

pub use native::NativeStorage;
pub use legacy::read_xor_token;

use anyhow::Result;
use std::path::PathBuf;

/// Trait for persisting the GitHub OAuth access token.
pub trait TokenStorage: Send + Sync {
    /// Store the GitHub access token.
    fn store_github_token(&self, token: &str) -> anyhow::Result<()>;

    /// Retrieve the stored GitHub access token.
    fn get_github_token(&self) -> anyhow::Result<String>;

    /// Delete the stored GitHub access token.
    fn delete_github_token(&self) -> anyhow::Result<()>;
}

/// Create storage for a specific profile.
///
/// Uses platform-native credential encryption (DPAPI on Windows, OS keyring
/// on macOS/Linux) via [`NativeStorage`]. The credential file is stored at
/// `credentials_path` (typically `<profile_dir>/github-copilot.json`).
///
/// If legacy XOR-encrypted credentials exist alongside the new path,
/// they are automatically migrated on first access.
pub fn create_storage_for_profile(
    credentials_path: PathBuf,
    profile_name: String,
) -> Result<Box<dyn TokenStorage>> {
    let storage = NativeStorage::new(credentials_path, profile_name)?;
    Ok(Box::new(storage))
}
