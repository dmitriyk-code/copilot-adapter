pub mod file;
pub mod keyring;

#[cfg(target_os = "windows")]
pub mod windows_credential;

use std::path::PathBuf;

/// Trait for persisting the GitHub OAuth access token.
pub trait TokenStorage {
    /// Store the GitHub access token.
    fn store_github_token(&self, token: &str) -> anyhow::Result<()>;

    /// Retrieve the stored GitHub access token.
    fn get_github_token(&self) -> anyhow::Result<String>;

    /// Delete the stored GitHub access token.
    fn delete_github_token(&self) -> anyhow::Result<()>;
}

/// Create the best available storage backend.
///
/// When `use_keyring` is false (the default), uses file-based storage at
/// `~/.copilot-adapter/credentials.json`. When `use_keyring` is true, tries
/// the OS keyring first and falls back to file storage if unavailable.
pub fn create_storage(use_keyring: bool) -> Box<dyn TokenStorage + Send + Sync> {
    create_storage_with_path(file::get_credentials_path(), use_keyring)
}

/// Create a storage backend with a specific file path.
///
/// This is the parameterized version used by profile support (Epic 5+).
/// The `path` is used for file-based storage; keyring storage ignores it.
///
/// **Note:** When `use_keyring` is true and the keyring is available, the `path`
/// parameter is not used — all keyring entries share the service name
/// `"copilot-adapter"`. Profile isolation via keyring would require separate
/// service names, not paths.
pub fn create_storage_with_path(
    path: PathBuf,
    use_keyring: bool,
) -> Box<dyn TokenStorage + Send + Sync> {
    if use_keyring {
        match keyring::KeyringStorage::new() {
            Ok(ks) => match ks.verify_available() {
                Ok(true) => {
                    tracing::info!("Using OS keyring for credential storage");
                    return Box::new(ks);
                }
                Ok(false) => {
                    tracing::warn!(
                        "OS keyring verification failed, falling back to file storage"
                    );
                }
                Err(e) => {
                    tracing::warn!(
                        error = %e,
                        "OS keyring verification error, falling back to file storage"
                    );
                }
            },
            Err(e) => {
                tracing::warn!(
                    error = %e,
                    "OS keyring not available, falling back to file storage"
                );
            }
        }
    }

    tracing::info!(path = %path.display(), "Using file-based credential storage");
    Box::new(file::FileStorage::with_path(path))
}
