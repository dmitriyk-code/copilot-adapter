# Multi-Instance Profiles — Design Document

**Status:** Proposed
**Date:** 2026-04-01
**Severity:** Low
**Related:** [HOME-DIR-STATUS.design.md](./HOME-DIR-STATUS.design.md), [HOME-DIR-TOKEN.design.md](./HOME-DIR-TOKEN.design.md)
**Prerequisite:** HOME-DIR-STATUS, HOME-DIR-TOKEN

---

## Executive Summary

The copilot-adapter currently supports a single running instance. This design introduces a **profile** concept — a named combination of port and GitHub credentials — enabling multiple concurrent instances. Profiles are entirely optional; the default profile ("default" on port 6767) preserves full backward compatibility. Profile data is stored under `~/.copilot-adapter/profiles/<name>/`, with each profile having its own `status.json` and `credentials.json`.

---

## Context / Background

### Current State
- Single instance: one PID file, one port, one credential store
- Port defaults to 6767, configurable via `-p`
- `status`/`stop` commands operate on the single instance
- No concept of named instances

### Target State
- Multiple concurrent instances, each identified by a profile name
- Profile = { name, port, github_token }
- Default profile "default" on port 6767 — zero-change for existing users
- CLI: `--profile <name>` / `-P <name>` on all management commands
- Directory structure:
  ```
  ~/.copilot-adapter/
  └── profiles/
      ├── default/
      │   ├── status.json
      │   └── credentials.json
      ├── work/
      │   ├── status.json
      │   └── credentials.json
      └── personal/
          └── ...
  ```

---

## Problem Statement

**Observed behavior:**
- Users with multiple GitHub accounts (personal + work) must stop/restart to switch
- CI/CD setups want dedicated instances on different ports
- Only one adapter instance can run at a time

**Expected behavior:**
- Multiple instances running concurrently, each with its own auth and port
- Easy management: `copilot-adapter start --profile work -p 8080`
- Clear visibility: `copilot-adapter status --all` shows all instances

---

## Goals and Non-Goals

### Goals

| ID | Goal | Success Criteria |
|----|------|------------------|
| G1 | Profile-scoped instances | Each profile has own status + credentials |
| G2 | Default profile backward compat | No flags needed for single-instance usage |
| G3 | CLI `--profile` flag on all commands | start, stop, status, auth, logout accept -P |
| G4 | `--all` for multi-instance operations | `status --all`, `stop --all` |
| G5 | Port conflict detection | Can't start two profiles on same port |
| G6 | Profile CRUD | `profiles list`, `profiles create`, `profiles delete` |
| G7 | Migrate existing data to default profile | Seamless upgrade |

### Non-Goals

| ID | Non-Goal | Rationale |
|----|----------|-----------|
| NG1 | Profile config file (port/settings per profile) | Start with CLI flags; config file later if needed |
| NG2 | Remote profile sync | Local-only; not a cloud feature |
| NG3 | Profile aliases or renaming | Keep it simple for v1 |
| NG4 | Shared credentials across profiles | Each profile has independent auth |

---

## Proposed Design

### Profile Data Model

```rust
// src/profile/types.rs

/// A named profile configuration.
#[derive(Debug, Clone)]
pub struct Profile {
    pub name: String,
    pub dir: PathBuf,
}

impl Profile {
    pub fn status_path(&self) -> PathBuf { self.dir.join("status.json") }
    pub fn credentials_path(&self) -> PathBuf { self.dir.join("credentials.json") }
}

/// Validates a profile name.
/// Rules: alphanumeric, dash, underscore. Max 64 chars. Not empty.
pub fn validate_profile_name(name: &str) -> Result<()> {
    if name.is_empty() || name.len() > 64 {
        anyhow::bail!("Profile name must be 1-64 characters");
    }
    if !name.chars().all(|c| c.is_alphanumeric() || c == '-' || c == '_') {
        anyhow::bail!("Profile name may only contain alphanumeric, dash, underscore");
    }
    Ok(())
}
```

### Profile Manager

```rust
// src/profile/mod.rs

pub const DEFAULT_PROFILE: &str = "default";

pub struct ProfileManager {
    base_dir: PathBuf, // ~/.copilot-adapter/profiles/
}

impl ProfileManager {
    pub fn new() -> Self {
        let base = dirs::home_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join(".copilot-adapter")
            .join("profiles");
        Self { base_dir: base }
    }

    pub fn get(&self, name: &str) -> Result<Profile> {
        validate_profile_name(name)?;
        Ok(Profile {
            name: name.to_string(),
            dir: self.base_dir.join(name),
        })
    }

    pub fn list(&self) -> Vec<String> {
        std::fs::read_dir(&self.base_dir)
            .into_iter()
            .flatten()
            .filter_map(|e| e.ok())
            .filter(|e| e.path().is_dir())
            .filter_map(|e| e.file_name().into_string().ok())
            .collect()
    }

    pub fn create(&self, name: &str) -> Result<Profile> {
        let profile = self.get(name)?;
        std::fs::create_dir_all(&profile.dir)?;
        Ok(profile)
    }

    pub fn delete(&self, name: &str) -> Result<()> {
        if name == DEFAULT_PROFILE {
            anyhow::bail!("Cannot delete the default profile");
        }
        let profile = self.get(name)?;
        if profile.dir.exists() {
            std::fs::remove_dir_all(&profile.dir)?;
        }
        Ok(())
    }

    /// Find profile by port (scanning all status files)
    pub fn find_by_port(&self, port: u16) -> Option<Profile> { /* ... */ }

    /// Check for port conflicts across all running profiles
    pub fn check_port_conflict(&self, port: u16, exclude: &str) -> Option<String> { /* ... */ }
}
```

### CLI Changes

```rust
// src/cli.rs additions

#[derive(Subcommand)]
pub enum Command {
    Start {
        #[arg(short = 'P', long, default_value = "default")]
        profile: String,
        // ... existing flags ...
    },
    Stop {
        #[arg(short = 'P', long, default_value = "default")]
        profile: String,
        #[arg(long)]
        all: bool,
    },
    Status {
        #[arg(short = 'P', long, default_value = "default")]
        profile: String,
        #[arg(long)]
        all: bool,
    },
    Auth {
        #[arg(short = 'P', long, default_value = "default")]
        profile: String,
        #[arg(long)]
        force: bool,
    },
    Logout {
        #[arg(short = 'P', long, default_value = "default")]
        profile: String,
    },
    /// Manage profiles
    Profiles {
        #[command(subcommand)]
        action: ProfileAction,
    },
}

#[derive(Subcommand)]
pub enum ProfileAction {
    List,
    Create { name: String },
    Delete { name: String },
}
```

### Profile Resolution Flow

```
User runs: copilot-adapter start --profile work -p 8080

1. Resolve profile: ProfileManager::get("work")
2. Create profile dir if needed: ~/.copilot-adapter/profiles/work/
3. Check port conflict: scan all profiles' status.json for port 8080
4. Load credentials: FileStorage::new(profile.credentials_path())
5. Run auth if needed (profile-scoped)
6. Write status: write_status_to(profile.status_path(), 8080)
7. Start server on port 8080
```

---

## Design Decisions

| Decision | Rationale |
|----------|-----------|
| Default profile "default" | 100% backward compatible; no flags needed |
| Profile = directory under profiles/ | Simple, discoverable, no separate config file |
| `--profile` / `-P` flag | Short and consistent across all commands |
| Can't delete default profile | Safety; always have a fallback |
| Port conflict detection | Prevent confusing bind errors |
| Profile name validation | Prevent filesystem issues; keep it simple |
| `profiles` subcommand | Clean separation from instance management |
| Each profile has own credentials | Users want different GitHub accounts per profile |

---

## File Changes Summary

| File | Change | Description |
|------|--------|-------------|
| `src/profile/mod.rs` | **New file** | ProfileManager, DEFAULT_PROFILE |
| `src/profile/types.rs` | **New file** | Profile struct, validation |
| `src/lib.rs` | Modified | Add `pub mod profile;` |
| `src/cli.rs` | Modified | Add --profile, --all, Profiles subcommand |
| `src/main.rs` | Modified | Profile resolution, pass to all commands |
| `src/storage/mod.rs` | Modified | Accept credentials path parameter |
| `src/storage/file.rs` | Modified | Accept custom path |
| `src/daemon/status.rs` | Modified | Accept custom status path |
| `src/daemon/mod.rs` | Modified | Profile-aware is_running, write_status |

---

## Testing Strategy

### Unit Tests
1. Profile name validation (valid, invalid, edge cases)
2. ProfileManager CRUD operations
3. Port conflict detection
4. Default profile behavior

### Integration Tests
1. Create profile → start → status → stop → delete
2. Two profiles on different ports simultaneously
3. Port conflict detection
4. Migration from non-profile to default profile

### Manual E2E Tests
1. `copilot-adapter profiles create work` → `auth -P work` → `start -P work -p 8080`
2. `copilot-adapter status --all` shows both default and work profiles
3. `copilot-adapter stop --all` stops all instances

---

## Risk Assessment

| Risk | Impact | Probability | Mitigation |
|------|--------|-------------|------------|
| Breaking backward compat | High | Low | Default profile = existing behavior; extensive testing |
| Port conflicts between profiles | Medium | Medium | Explicit check at startup |
| Filesystem race conditions | Low | Low | Single-user tool; minimal concurrency |
| Profile name edge cases | Low | Low | Strict validation regex |
| Complex migration | Medium | Low | Simple copy from old paths to default profile |

---

## Success Criteria

1. `copilot-adapter start` works identically to today (default profile)
2. Multiple profiles can run concurrently on different ports
3. `status --all` shows all running instances
4. `stop --all` stops all instances
5. Profile CRUD commands work
6. Port conflict detection prevents duplicate binds
7. Migration from single-instance to default profile is seamless

---

## References

- [HOME-DIR-STATUS.design.md](./HOME-DIR-STATUS.design.md) — Status file under ~/.copilot-adapter/
- [HOME-DIR-TOKEN.design.md](./HOME-DIR-TOKEN.design.md) — Token file under ~/.copilot-adapter/
- `src/daemon/mod.rs` — Current single-instance PID management
- `src/storage/mod.rs` — Current credential storage selection
- `src/cli.rs` — Current CLI argument definitions
