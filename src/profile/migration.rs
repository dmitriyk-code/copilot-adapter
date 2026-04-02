//! Auto-migration from flat directory layout to profile-based layout.
//!
//! Migrates `~/.copilot-adapter/status.json` and `credentials.json` to
//! `~/.copilot-adapter/profiles/default/` and handles legacy temp dir PID files.
//!
//! All operations are idempotent — safe to run multiple times without data loss.
//! Migration is a no-op once `<base_dir>/profiles/` exists.

use std::path::Path;

/// Auto-migrate from flat directory layout to profile-based layout.
///
/// Called at startup before any [`super::ProfileManager`] operations.
/// Uses the real base directory (`~/.copilot-adapter/`) and legacy PID path.
///
/// This is a no-op if `~/.copilot-adapter/profiles/` already exists.
pub fn migrate_to_profiles() {
    let base_dir = crate::daemon::status::get_base_dir();
    let legacy_pid_path = crate::daemon::get_pid_path();
    run_migration(&base_dir, Some(&legacy_pid_path));
}

/// Run migration with explicit paths (for testing).
///
/// If `base_dir/profiles/` doesn't exist but `base_dir/status.json` or
/// `base_dir/credentials.json` does:
/// 1. Create `base_dir/profiles/default/`
/// 2. Move `status.json` and `credentials.json` into it
/// 3. Migrate legacy temp dir PID file to profile status
///
/// Pass `legacy_pid_path` as `None` to skip legacy PID file migration.
pub fn run_migration(base_dir: &Path, legacy_pid_path: Option<&Path>) {
    let profiles_dir = base_dir.join("profiles");

    // If profiles/ already exists, migration is complete or not needed
    if profiles_dir.exists() {
        return;
    }

    let flat_status = base_dir.join("status.json");
    let flat_credentials = base_dir.join("credentials.json");

    let has_flat_status = flat_status.exists();
    let has_flat_credentials = flat_credentials.exists();
    let has_legacy_pid = legacy_pid_path.is_some_and(|p| p.exists());

    if !has_flat_status && !has_flat_credentials && !has_legacy_pid {
        // Nothing to migrate
        return;
    }

    // Create the default profile directory
    let default_dir = profiles_dir.join("default");
    if let Err(e) = std::fs::create_dir_all(&default_dir) {
        eprintln!(
            "Warning: failed to create default profile directory for migration: {e}"
        );
        return;
    }

    eprintln!("Migrating data to profile-based layout...");

    // Move flat-dir status.json → profiles/default/status.json
    if has_flat_status {
        move_file(&flat_status, &default_dir.join("status.json"));
    }

    // Move flat-dir credentials.json → profiles/default/credentials.json
    if has_flat_credentials {
        move_file(&flat_credentials, &default_dir.join("credentials.json"));
    }

    // Migrate legacy temp dir PID file
    if let Some(pid_path) = legacy_pid_path {
        if pid_path.exists() {
            migrate_legacy_pid_file(pid_path, &default_dir);
        }
    }
}

/// Move a file from `src` to `dest`, trying rename first and falling back
/// to copy+delete for cross-device moves.
///
/// No-op if `dest` already exists (idempotent safety).
fn move_file(src: &Path, dest: &Path) {
    if dest.exists() {
        return;
    }

    match std::fs::rename(src, dest) {
        Ok(()) => {
            eprintln!(
                "  Migrated {} → {}",
                src.file_name().unwrap_or_default().to_string_lossy(),
                dest.display()
            );
        }
        Err(_rename_err) => {
            // Fallback to copy+delete (e.g., cross-device move)
            match std::fs::copy(src, dest) {
                Ok(_) => {
                    let _ = std::fs::remove_file(src);
                    eprintln!(
                        "  Migrated {} → {}",
                        src.file_name().unwrap_or_default().to_string_lossy(),
                        dest.display()
                    );
                }
                Err(copy_err) => {
                    eprintln!(
                        "  Warning: failed to migrate {}: {copy_err}",
                        src.display()
                    );
                }
            }
        }
    }
}

/// Migrate a legacy temp dir PID file to a profile status file.
///
/// If a running process is found, synthesizes a `status.json` in the profile
/// directory. If the process is dead, cleans up the stale files.
fn migrate_legacy_pid_file(pid_path: &Path, profile_dir: &Path) {
    let status_dest = profile_dir.join("status.json");
    let port_path = pid_path.with_extension("port");

    // If status.json was already migrated from flat dir, just clean up legacy files
    if status_dest.exists() {
        let _ = std::fs::remove_file(pid_path);
        let _ = std::fs::remove_file(&port_path);
        eprintln!("  Cleaned up legacy PID file (status already migrated)");
        return;
    }

    let content = match std::fs::read_to_string(pid_path) {
        Ok(c) => c,
        Err(_) => return,
    };

    let pid: u32 = match content.trim().parse() {
        Ok(p) => p,
        Err(_) => {
            // Invalid PID file — clean up both PID and port files
            let _ = std::fs::remove_file(pid_path);
            let _ = std::fs::remove_file(&port_path);
            return;
        }
    };

    if !crate::daemon::process_exists(pid) {
        // Process is dead — clean up stale legacy files
        let _ = std::fs::remove_file(pid_path);
        let _ = std::fs::remove_file(&port_path);
        eprintln!("  Cleaned up stale legacy PID file");
        return;
    }

    // Process is running — synthesize a status.json from legacy PID/port data
    let port = std::fs::read_to_string(&port_path)
        .ok()
        .and_then(|s| s.trim().parse::<u16>().ok())
        .unwrap_or(0);

    let status = crate::daemon::status::StatusFile {
        pid,
        port,
        started_at: None,
        version: None,
    };

    match serde_json::to_string_pretty(&status) {
        Ok(json) => {
            if let Err(e) = std::fs::write(&status_dest, json) {
                eprintln!("  Warning: failed to write migrated status file: {e}");
                return;
            }
            // Clean up legacy files after successful synthesis
            let _ = std::fs::remove_file(pid_path);
            let _ = std::fs::remove_file(&port_path);
            eprintln!("  Migrated legacy PID file to default profile status");
        }
        Err(e) => {
            eprintln!("  Warning: failed to serialize status for migration: {e}");
        }
    }
}
