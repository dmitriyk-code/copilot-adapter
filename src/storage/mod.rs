pub mod file;
pub mod keyring;

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
/// Tries OS keyring first, falls back to encrypted file storage.
pub fn create_storage() -> Box<dyn TokenStorage + Send + Sync> {
    match keyring::KeyringStorage::new() {
        Ok(ks) => {
            // Verify keyring works with a round-trip test
            match ks.verify_available() {
                Ok(true) => {
                    tracing::debug!("Using OS keyring for token storage");
                    Box::new(ks)
                }
                Ok(false) => {
                    tracing::info!(
                        "OS keyring verification failed, falling back to encrypted file storage"
                    );
                    Box::new(file::FileStorage::new())
                }
                Err(e) => {
                    tracing::info!(
                        error = %e,
                        "OS keyring verification error, falling back to encrypted file storage"
                    );
                    Box::new(file::FileStorage::new())
                }
            }
        }
        Err(e) => {
            tracing::info!(
                error = %e,
                "OS keyring not available, falling back to encrypted file storage"
            );
            Box::new(file::FileStorage::new())
        }
    }
}
