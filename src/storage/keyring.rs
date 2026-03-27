use anyhow::{anyhow, Result};

use super::TokenStorage;

const SERVICE_NAME: &str = "copilot-adapter";
const GITHUB_TOKEN_KEY: &str = "github_token";
const VERIFY_KEY: &str = "__keyring_check__";

/// Token storage backed by the OS keyring (Keychain, Credential Manager, Secret Service).
pub struct KeyringStorage {
    service: String,
}

impl KeyringStorage {
    pub fn new() -> Result<Self> {
        Ok(Self {
            service: SERVICE_NAME.to_string(),
        })
    }

    /// Test whether the keyring is usable by writing and deleting a probe entry.
    pub fn verify_available(&self) -> Result<bool> {
        let entry = keyring::Entry::new(&self.service, VERIFY_KEY)?;
        match entry.set_password("probe") {
            Ok(()) => {
                let _ = entry.delete_password();
                Ok(true)
            }
            Err(_) => Ok(false),
        }
    }
}

impl TokenStorage for KeyringStorage {
    fn store_github_token(&self, token: &str) -> Result<()> {
        let entry = keyring::Entry::new(&self.service, GITHUB_TOKEN_KEY)
            .map_err(|e| anyhow!("Failed to create keyring entry: {e}"))?;
        entry
            .set_password(token)
            .map_err(|e| anyhow!("Failed to store token in keyring: {e}"))
    }

    fn get_github_token(&self) -> Result<String> {
        let entry = keyring::Entry::new(&self.service, GITHUB_TOKEN_KEY)
            .map_err(|e| anyhow!("Failed to create keyring entry: {e}"))?;
        entry
            .get_password()
            .map_err(|e| anyhow!("Failed to get token from keyring: {e}"))
    }

    fn delete_github_token(&self) -> Result<()> {
        let entry = keyring::Entry::new(&self.service, GITHUB_TOKEN_KEY)
            .map_err(|e| anyhow!("Failed to create keyring entry: {e}"))?;
        match entry.delete_password() {
            Ok(()) => Ok(()),
            // Not finding the credential is fine — it's already gone
            Err(keyring::Error::NoEntry) => Ok(()),
            Err(e) => Err(anyhow!("Failed to delete token from keyring: {e}")),
        }
    }
}
