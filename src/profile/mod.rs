//! Profile management for multi-instance support.
//!
//! Each profile has its own directory under `~/.copilot-adapter/profiles/<name>/`
//! containing `status.json` and `credentials.json`.

pub mod types;

pub use types::{validate_profile_name, Profile};

use crate::daemon::status::{get_base_dir, read_status_from};
use anyhow::Result;
use std::path::PathBuf;

/// The default profile name used when no `--profile` flag is given.
pub const DEFAULT_PROFILE: &str = "default";

/// Manages named profiles under the base directory.
pub struct ProfileManager {
    base_dir: PathBuf,
}

impl Default for ProfileManager {
    fn default() -> Self {
        Self::new()
    }
}

impl ProfileManager {
    /// Create a new ProfileManager rooted at `get_base_dir()`.
    pub fn new() -> Self {
        Self {
            base_dir: get_base_dir(),
        }
    }

    /// Create a ProfileManager rooted at a custom base directory.
    pub fn with_base_dir(base_dir: PathBuf) -> Self {
        Self { base_dir }
    }

    /// Directory containing all profile subdirectories.
    fn profiles_dir(&self) -> PathBuf {
        self.base_dir.join("profiles")
    }

    /// Resolve a profile by name.
    ///
    /// For the `"default"` profile the directory is created automatically if it
    /// doesn't already exist. For other profiles the directory must exist (use
    /// [`create`] first).
    pub fn get(&self, name: &str) -> Result<Profile> {
        validate_profile_name(name)?;

        let dir = self.profiles_dir().join(name);

        if name == DEFAULT_PROFILE {
            std::fs::create_dir_all(&dir)?;
        } else if !dir.exists() {
            anyhow::bail!("Profile '{}' does not exist", name);
        }

        Ok(Profile {
            name: name.to_string(),
            dir,
        })
    }

    /// List all profiles by reading subdirectories of `<base_dir>/profiles/`.
    ///
    /// Only directories with valid profile names are returned; any directory
    /// whose name fails [`validate_profile_name`] is silently skipped.
    pub fn list(&self) -> Vec<Profile> {
        let profiles_dir = self.profiles_dir();
        let entries = match std::fs::read_dir(&profiles_dir) {
            Ok(e) => e,
            Err(_) => return Vec::new(),
        };

        entries
            .filter_map(|entry| {
                let entry = entry.ok()?;
                if !entry.file_type().ok()?.is_dir() {
                    return None;
                }
                let name = entry.file_name().to_string_lossy().to_string();
                // Skip directories with names that fail validation
                if validate_profile_name(&name).is_err() {
                    return None;
                }
                Some(Profile {
                    name,
                    dir: entry.path(),
                })
            })
            .collect()
    }

    /// Create a new profile directory and return the profile.
    ///
    /// The `"default"` profile cannot be explicitly created; use [`get`] instead,
    /// which lazily creates it on first access.
    pub fn create(&self, name: &str) -> Result<Profile> {
        validate_profile_name(name)?;

        if name == DEFAULT_PROFILE {
            anyhow::bail!("Cannot explicitly create the default profile; use get(\"default\") instead");
        }

        let dir = self.profiles_dir().join(name);
        if dir.exists() {
            anyhow::bail!("Profile '{}' already exists", name);
        }

        std::fs::create_dir_all(&dir)?;

        Ok(Profile {
            name: name.to_string(),
            dir,
        })
    }

    /// Delete a profile directory recursively.
    ///
    /// The `"default"` profile cannot be deleted.
    pub fn delete(&self, name: &str) -> Result<()> {
        validate_profile_name(name)?;

        if name == DEFAULT_PROFILE {
            anyhow::bail!("Cannot delete the default profile");
        }

        let dir = self.profiles_dir().join(name);
        if !dir.exists() {
            anyhow::bail!("Profile '{}' does not exist", name);
        }

        std::fs::remove_dir_all(&dir)?;
        Ok(())
    }

    /// Find the profile whose `status.json` reports the given port.
    pub fn find_by_port(&self, port: u16) -> Option<Profile> {
        for profile in self.list() {
            if let Some(status) = read_status_from(&profile.status_path()) {
                if status.port == port {
                    return Some(profile);
                }
            }
        }
        None
    }

    /// Check that no *other* profile is already using the given port.
    ///
    /// Returns `Ok(())` if the port is free or only used by `current_profile`.
    /// Returns an error if another profile has a `status.json` claiming that port.
    pub fn check_port_conflict(&self, port: u16, current_profile: &str) -> Result<()> {
        for profile in self.list() {
            if profile.name == current_profile {
                continue;
            }
            if let Some(status) = read_status_from(&profile.status_path()) {
                if status.port == port {
                    anyhow::bail!(
                        "Port {} is already in use by profile '{}'",
                        port,
                        profile.name
                    );
                }
            }
        }
        Ok(())
    }
}
