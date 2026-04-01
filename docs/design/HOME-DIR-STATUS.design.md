# Home Directory Status File — Design Document

**Status:** Proposed
**Date:** 2026-04-01
**Severity:** Low
**Related:** [MULTI-INSTANCE-PROFILES.design.md](./MULTI-INSTANCE-PROFILES.design.md)

---

## Executive Summary

The copilot-adapter currently stores its PID and port files in the OS temp directory (`/tmp/` on Unix, `%TEMP%` on Windows). This is unreliable — temp directories may be cleaned periodically, paths vary across user contexts, and the location is inconsistent with where credentials are already stored. This design proposes moving runtime status to a single JSON file at `~/.copilot-adapter/status.json`, with automatic fallback to the temp directory if the home directory is not writable.

---

## Context / Background

### Current State

Runtime state is managed in `src/daemon/mod.rs` via two separate files:
- **PID file:** `std::env::temp_dir() / "copilot-adapter.pid"` — contains process ID as text
- **Port file:** `std::env::temp_dir() / "copilot-adapter.port"` — contains port number as text

Functions: `write_pid_file()`, `is_running()`, `remove_pid_file()`, `write_port_file()`, `read_port()`, `remove_port_file()`, `process_exists()`

### Target State

A single JSON status file at `~/.copilot-adapter/status.json`:
```json
{
  "pid": 12345,
  "port": 6767,
  "started_at": "2026-04-01T10:30:00.000000000+00:00",
  "version": "0.1.0"
}
```

With backward compatibility: check both old temp location and new home location during transition.

---

## Problem Statement

**Observed behavior:**
- PID/port files stored in temp directory which may be cleaned on reboot
- Different temp paths for different user contexts (e.g., elevated vs normal)
- No standard location — external tools can't reliably find the adapter
- Inconsistent with credential storage (`~/.config/copilot-adapter/`)

**Expected behavior:**
- Status stored in a well-known, persistent location under the user's home directory
- Single file with structured data (JSON) instead of two plain-text files
- Richer metadata (start time, version) for better diagnostics

**Impact:**
- Affects all users on systems with temp directory cleanup
- `status` command may report incorrect state after reboot

---

## Goals and Non-Goals

### Goals

| ID | Goal | Success Criteria |
|----|------|------------------|
| G1 | Store status in `~/.copilot-adapter/status.json` | File created on start, readable by status command |
| G2 | Single JSON file with structured data | PID, port, started_at, version in one file |
| G3 | Auto-create directory on first write | `~/.copilot-adapter/` created if missing |
| G4 | Handle stale files (process died) | `is_running` detects dead processes, auto-cleans |
| G5 | Fallback to temp dir if home not writable | Graceful degradation, not failure |
| G6 | Backward compatible during transition | Check both old and new locations |
| G7 | Handle corrupt JSON gracefully | Treat as no status, recreate |

### Non-Goals

| ID | Non-Goal | Rationale |
|----|----------|-----------|
| NG1 | Multi-instance support | Deferred to MULTI-INSTANCE-PROFILES |
| NG2 | File locking | Single instance; race conditions unlikely |
| NG3 | Migrate old PID files automatically | Just check both locations; old files will be cleaned up naturally |

---

## Research / Analysis

### Options Considered

#### Option A: Home dotdir `~/.copilot-adapter/` (Recommended)

**Pros:** Standard convention (Docker, Cargo, npm), persistent, discoverable, shareable with credentials
**Cons:** Requires home dir access; may fail in containers

#### Option B: XDG runtime dir (`$XDG_RUNTIME_DIR`)

**Pros:** Designed for runtime state
**Cons:** Not available on all systems; cleaned on logout; not available on Windows/macOS

#### Option C: Platform-specific config dir

**Pros:** Follows OS conventions (`%APPDATA%`, `~/Library/Application Support/`)
**Cons:** Different paths per OS; runtime state doesn't belong in config dirs

#### Option D: Keep temp dir (Status Quo)

**Pros:** No changes needed
**Cons:** All the problems described above persist

### Recommended Approach

**Option A** — `~/.copilot-adapter/` is the best fit. It's a well-known convention, persistent across reboots, consistent across platforms, and shareable with the credential storage planned in HOME-DIR-TOKEN.

---

## Proposed Design

### StatusFile Struct

```rust
// src/daemon/status.rs

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Runtime status of the copilot-adapter daemon.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StatusFile {
    pub pid: u32,
    pub port: u16,
    pub started_at: String,  // ISO 8601
    pub version: String,
}
```

### Directory Resolution

```rust
/// Returns the status directory path, creating it if needed.
/// Falls back to temp dir if home is not writable.
pub fn get_status_dir() -> PathBuf {
    if let Some(home) = dirs::home_dir() {
        let dir = home.join(".copilot-adapter");
        if std::fs::create_dir_all(&dir).is_ok() {
            return dir;
        }
    }
    // Fallback to temp directory
    std::env::temp_dir()
}

pub fn get_status_path() -> PathBuf {
    get_status_dir().join("status.json")
}
```

### Write/Read/Cleanup

```rust
pub fn write_status(port: u16) -> anyhow::Result<()> {
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
    // Check new location first
    if let Some(status) = read_status() {
        if process_exists(status.pid) {
            return Some(status);
        }
        // Stale — clean up
        remove_status();
    }
    // Legacy fallback: check temp dir PID file
    let pid_path = get_pid_path(); // legacy temp dir path
    if let Ok(content) = std::fs::read_to_string(&pid_path) {
        if let Ok(pid) = content.trim().parse::<u32>() {
            if process_exists(pid) {
                let port = read_port().unwrap_or(0);
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

---

## Design Decisions

| Decision | Rationale |
|----------|-----------|
| `~/.copilot-adapter/` not `~/.config/copilot-adapter/` | Simpler, more discoverable; aligns with Docker, npm conventions |
| Single JSON file, not separate PID/port files | Atomic writes, richer metadata, single parse |
| `chrono::Utc::now()` for started_at | Already a dependency; ISO 8601 is standard |
| `env!("CARGO_PKG_VERSION")` for version | Compile-time, zero cost, always accurate |
| Fallback to temp dir | Graceful degradation for containers/restricted environments |
| Legacy backward compat | Smooth transition; old files cleaned up naturally |
| No file locking | Single-instance design; multi-instance deferred to profiles feature |
| `serde_json::to_string_pretty` | Human-readable for debugging |

---

## File Changes Summary

| File | Change | Description |
|------|--------|-------------|
| `src/daemon/status.rs` | **New file** | StatusFile struct, get_status_dir/path, read/write/remove/is_running |
| `src/daemon/mod.rs` | Modified | Delegate to new status functions, deprecate old ones, add remove_all_status_files() |
| `src/daemon/unix.rs` | Modified | Use remove_all_status_files() in stop_daemon() |
| `src/daemon/windows.rs` | Modified | Use remove_all_status_files() in stop_daemon() |
| `src/server.rs` | Modified | Use write_status()/remove_all_status_files() |
| `src/main.rs` | Modified | Use is_running_from_status() for richer status output |

---

## Testing Strategy

### Unit Tests
1. StatusFile serialization/deserialization round-trip
2. get_status_dir() returns valid path
3. write_status() creates directory and file
4. read_status() handles missing/corrupt files
5. is_running_from_status() detects stale PIDs

### Integration Tests
1. Full lifecycle: write → is_running → remove → not running
2. Stale file detection and cleanup
3. Legacy PID file backward compatibility

### Manual E2E Tests
1. Start daemon → verify `~/.copilot-adapter/status.json` exists
2. `copilot-adapter status` shows PID, port, version, start time
3. Stop daemon → verify status file removed
4. Crash simulation (kill -9) → status file stale → next `status` auto-cleans

---

## Risk Assessment

| Risk | Impact | Probability | Mitigation |
|------|--------|-------------|------------|
| Home dir not writable (containers) | Medium | Low | Automatic fallback to temp dir |
| Stale file after crash | Low | Medium | is_running_from_status() checks process_exists() |
| Corrupt JSON after partial write | Low | Very Low | read_status() returns None, recreated on next start |
| Legacy PID files left behind | Low | Medium | Backward compat checks both locations |
| `dirs` crate not available | Low | Very Low | Already used by file.rs for credential storage paths |

---

## Success Criteria

1. `~/.copilot-adapter/status.json` created on daemon start
2. `copilot-adapter status` displays PID, port, version, and start time
3. Stale files auto-cleaned when process is dead
4. Legacy PID files still detected during transition
5. Fallback to temp dir works when home is not writable
6. All tests pass: `cargo test`

---

## References

- `src/daemon/mod.rs` — Current PID/port file management
- `src/storage/file.rs` — Existing home directory credential storage (path resolution pattern)
- [MULTI-INSTANCE-PROFILES.design.md](./MULTI-INSTANCE-PROFILES.design.md) — Future multi-instance support
- [`dirs` crate](https://docs.rs/dirs/latest/dirs/) — Cross-platform home directory resolution
