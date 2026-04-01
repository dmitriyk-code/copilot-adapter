# Home Directory Status File — Implementation Plan

**Status:** Not Started
**Date:** 2026-04-01
**Based on:** [HOME-DIR-STATUS.design.md](./HOME-DIR-STATUS.design.md)
**Prerequisite:** None
**Estimated Time:** 1-2 days

---

## Executive Summary

This plan implements the migration of runtime status storage from temp directory PID/port files to a single `~/.copilot-adapter/status.json` file. The implementation is organized into 6 epics: types/directory, read/write functions, daemon module migration, caller updates, testing, and documentation.

---

## Implementation Plan

### Epic 1: Status File Types and Directory Resolution

**Status:** Not Started
**Objective:** Create the StatusFile struct and directory resolution logic.

#### Task 1.1: Create status module

**File:** `src/daemon/status.rs` (NEW)

```rust
use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StatusFile {
    pub pid: u32,
    pub port: u16,
    pub started_at: String,
    pub version: String,
}

pub fn get_status_dir() -> PathBuf {
    if let Some(home) = dirs::home_dir() {
        let dir = home.join(".copilot-adapter");
        if std::fs::create_dir_all(&dir).is_ok() {
            return dir;
        }
    }
    std::env::temp_dir()
}

pub fn get_status_path() -> PathBuf {
    get_status_dir().join("status.json")
}

pub fn write_status(port: u16) -> Result<()> {
    let status = StatusFile {
        pid: std::process::id(),
        port,
        started_at: chrono::Utc::now().to_rfc3339(),
        version: env!("CARGO_PKG_VERSION").to_string(),
    };
    let path = get_status_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&path, serde_json::to_string_pretty(&status)?)?;
    Ok(())
}

pub fn read_status() -> Option<StatusFile> {
    let path = get_status_path();
    let content = std::fs::read_to_string(&path).ok()?;
    serde_json::from_str(&content).ok()
}

pub fn remove_status() {
    let _ = std::fs::remove_file(get_status_path());
}

pub fn is_running_from_status() -> Option<StatusFile> {
    if let Some(status) = read_status() {
        if super::process_exists(status.pid) {
            return Some(status);
        }
        remove_status();
    }
    // Legacy fallback
    let pid_path = super::get_pid_path();
    if let Ok(content) = std::fs::read_to_string(&pid_path) {
        if let Ok(pid) = content.trim().parse::<u32>() {
            if super::process_exists(pid) {
                let port = super::read_port().unwrap_or(0);
                return Some(StatusFile {
                    pid, port,
                    started_at: "unknown".to_string(),
                    version: "unknown".to_string(),
                });
            }
            let _ = std::fs::remove_file(&pid_path);
        }
    }
    None
}
```

**Acceptance Criteria:**
- [ ] StatusFile struct with serde derives
- [ ] get_status_dir() with home dir + fallback
- [ ] write_status()/read_status()/remove_status()/is_running_from_status()
- [ ] Legacy backward compatibility

#### Task 1.2: Register module

**File:** `src/daemon/mod.rs` (MODIFIED)

Add `mod status;` and `pub use status::*;`

---

### Epic 2: Migrate daemon/mod.rs

**Status:** Not Started
**Objective:** Update public API to use new status functions.

#### Task 2.1: Update is_running()

```rust
pub fn is_running() -> Option<u32> {
    is_running_from_status().map(|s| s.pid)
}
```

#### Task 2.2: Update read_port()

```rust
pub fn read_port() -> Option<u16> {
    if let Some(status) = read_status() {
        if status.port > 0 { return Some(status.port); }
    }
    // Legacy fallback
    let port_path = get_pid_path().with_extension("port");
    std::fs::read_to_string(&port_path).ok()?.trim().parse().ok()
}
```

#### Task 2.3: Add remove_all_status_files()

```rust
pub fn remove_all_status_files() {
    remove_status();
    let _ = std::fs::remove_file(get_pid_path());
    let _ = std::fs::remove_file(get_pid_path().with_extension("port"));
}
```

#### Task 2.4: Update unix.rs and windows.rs

Replace `remove_pid_file(); remove_port_file();` with `super::remove_all_status_files();` in both `stop_daemon()` functions.

---

### Epic 3: Update Callers

**Status:** Not Started

#### Task 3.1: Update server.rs

**Before:**
```rust
if write_pid {
    crate::daemon::write_pid_file()?;
    crate::daemon::write_port_file(port)?;
}
// ... shutdown ...
if write_pid {
    crate::daemon::remove_pid_file();
    crate::daemon::remove_port_file();
}
```

**After:**
```rust
if write_pid {
    crate::daemon::write_status(port)?;
}
// ... shutdown ...
if write_pid {
    crate::daemon::remove_all_status_files();
}
```

#### Task 3.2: Update main.rs Status command

**After:**
```rust
Command::Status => match daemon::is_running_from_status() {
    Some(status) => {
        println!("Adapter running on PID {}, port {}", status.pid, status.port);
        if status.version != "unknown" {
            println!("  Version:  {}", status.version);
        }
        if status.started_at != "unknown" {
            println!("  Started:  {}", status.started_at);
        }
    }
    None => println!("Adapter is not running."),
},
```

---

### Epic 4: Testing

**Status:** Not Started

#### Task 4.1: Unit tests for StatusFile

Test serialization/deserialization, directory resolution, stale file cleanup.

#### Task 4.2: Update integration daemon_tests.rs

Update to use `write_status(port)`, `is_running_from_status()`, `remove_all_status_files()`. Add backward compatibility test with legacy PID file.

---

### Epic 5: Documentation

**Status:** Not Started

Update CLAUDE.md (add status.rs to project structure, add development note), e2e-testing.md (richer status output), BACKLOG.md (move to Done).

---

## File Changes Summary

| File | Change | Description |
|------|--------|-------------|
| `src/daemon/status.rs` | **New file** | StatusFile, directory resolution, read/write/cleanup |
| `src/daemon/mod.rs` | Modified | Register module, delegate to new functions, add remove_all_status_files() |
| `src/daemon/unix.rs` | Modified | Use remove_all_status_files() |
| `src/daemon/windows.rs` | Modified | Use remove_all_status_files() |
| `src/server.rs` | Modified | Use write_status()/remove_all_status_files() |
| `src/main.rs` | Modified | Richer status output with version/start time |
| `tests/integration/daemon_tests.rs` | Modified | Update for new API |
| `CLAUDE.md` | Modified | Add status.rs to structure, add note |
| `docs/design/BACKLOG.md` | Modified | Move item to Done |

---

## Risk Assessment

| Risk | Impact | Probability | Mitigation |
|------|--------|-------------|------------|
| Home dir not writable | Medium | Low | Fallback to temp dir |
| Legacy PID files left behind | Low | Medium | Backward compat in is_running_from_status() |
| dirs crate unavailable | Low | Very Low | Already used by file.rs |

---

## Success Criteria

1. Status file at `~/.copilot-adapter/status.json` after start
2. `copilot-adapter status` shows PID, port, version, start time
3. Legacy PID files detected during transition
4. `cargo test` passes with zero failures

---

## References

- [HOME-DIR-STATUS.design.md](./HOME-DIR-STATUS.design.md)
- [BACKLOG.md](./BACKLOG.md)
- [`dirs` crate](https://docs.rs/dirs/latest/dirs/)
