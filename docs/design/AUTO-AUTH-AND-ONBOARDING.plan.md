# Auto-Auth and Onboarding Improvements

## Overview

This plan covers improvements to the first-run experience for copilot-adapter:

1. **Auto-auth on missing token** — Automatically start auth flow if no credentials exist
2. **Interactive browser launch** — Offer to open auth URL in browser with user confirmation
3. **Post-start guidance** — Display copy-paste-able environment variable commands
4. **Persistence hints** — Show how to persist settings permanently
5. **Documentation updates** — Update README.md with new features and Claude Code settings.json option

## Research Findings

### Claude Code Settings Locations

Based on official Anthropic documentation:

| Location | Scope | Precedence |
|----------|-------|------------|
| `~/.claude/settings.json` | User-level (all projects) | Lower |
| `<project>/.claude/settings.json` | Project-level | Higher (overrides user) |
| `<project>/.claude/settings.local.json` | Project-level, gitignored | Highest |

### Settings.json Structure

Claude Code settings.json supports an `env` block for environment variables:

```json
{
  "env": {
    "ANTHROPIC_BASE_URL": "http://127.0.0.1:6767",
    "ANTHROPIC_API_KEY": "dummy"
  }
}
```

**Important:** The `env` block sets environment variables that Claude Code will use. This is the recommended way to configure the adapter for Claude Code users.

### Current Local Settings Example

From `.claude/settings.local.json` in this project:
```json
{
  "permissions": {
    "allow": [...],
    "deny": [...]
  }
}
```

This confirms the JSON structure and that `env` can be added alongside other settings.

---

## Implementation Plan

### Epic 1: Auto-Auth on Missing Token

**Status: DONE**

#### 1.1 Add `--skip-auth` CLI flag — **DONE**

**File:** `src/cli.rs`

Add a new flag to the `Start` command:

```rust
#[derive(Subcommand, Debug)]
pub enum Command {
    Start {
        // ... existing flags ...

        /// Skip automatic authentication if not logged in
        #[arg(long)]
        skip_auth: bool,
    },
    // ...
}
```

**Rationale:** Some CI/CD or automated environments may want to fail fast rather than prompt for auth.

#### 1.2 Implement auth check before server start — **DONE**

**File:** `src/main.rs`

Modify the `Command::Start` handler:

```rust
Command::Start { daemon, port, host, ..., skip_auth } => {
    // ... existing "already running" check ...

    // NEW: Check authentication status before starting
    if !skip_auth {
        let store = storage::create_storage();
        let has_token = store.get_github_token().is_ok();

        if !has_token {
            // If daemon mode, we MUST authenticate before daemonizing
            // because daemon process can't do interactive auth
            if is_daemon {
                eprintln!("No authentication credentials found.");
                eprintln!("Please run 'copilot-adapter auth' first, or use --skip-auth to bypass.");
                std::process::exit(1);
            }

            // Foreground mode: offer to authenticate now
            eprintln!("No authentication credentials found.");
            eprintln!("Starting authentication flow...\n");

            // Reuse existing auth logic
            run_auth(false).await?;
        }
    }

    // ... rest of existing start logic ...
}
```

#### 1.3 Validate token on start (not just existence) — **DONE**

Enhance the check to verify the token is actually valid:

```rust
// After checking has_token, also verify it works
let auth_client = DeviceFlowAuth::new();
let manager = TokenManager::new(store, auth_client).await?;

match manager.get_valid_token().await {
    Ok(_) => {
        // Token is valid, proceed
    }
    Err(e) => {
        if !skip_auth {
            eprintln!("Stored token is invalid or expired: {e}");
            if is_daemon {
                eprintln!("Please run 'copilot-adapter auth --force' first.");
                std::process::exit(1);
            }
            eprintln!("Starting re-authentication...\n");
            run_auth(true).await?; // force=true
        }
    }
}
```

#### 1.4 Update help text — **DONE**

**File:** `src/cli.rs`

Update command descriptions:

```rust
/// Start the adapter server
///
/// If not authenticated, will prompt for authentication in foreground mode.
/// In daemon mode, authentication must be completed first.
Start { ... }
```

**Completion Notes:**
- Windows daemon child always receives `--skip-auth` unconditionally (critical fix for zombie-process risk)
- Pre-validated `Arc<TokenManager>` is reused by server on foreground/Unix paths (eliminates redundant token exchange)
- Tracing gap during pre-start auth check documented with inline comment
- All 246 unit tests and 76 integration tests pass

---

### Epic 2: Interactive Browser Launch

**Status: DONE**

#### 2.1 Add browser-opening utility

**File:** `src/auth/browser.rs` (new file)

```rust
use std::process::Command;

/// Attempt to open a URL in the system's default browser.
/// Returns Ok(true) if successful, Ok(false) if no browser available, Err on failure.
pub fn open_url(url: &str) -> anyhow::Result<bool> {
    #[cfg(target_os = "windows")]
    {
        Command::new("cmd")
            .args(["/C", "start", "", url])
            .spawn()?;
        Ok(true)
    }

    #[cfg(target_os = "macos")]
    {
        Command::new("open")
            .arg(url)
            .spawn()?;
        Ok(true)
    }

    #[cfg(target_os = "linux")]
    {
        // Try xdg-open first (most common), then fallback to common browsers
        if Command::new("xdg-open").arg(url).spawn().is_ok() {
            return Ok(true);
        }
        // Could also try: sensible-browser, x-www-browser, firefox, chromium
        Ok(false)
    }

    #[cfg(not(any(target_os = "windows", target_os = "macos", target_os = "linux")))]
    {
        Ok(false)
    }
}
```

#### 2.2 Add non-blocking stdin check utility

**File:** `src/auth/input.rs` (new file)

We need to detect if the user pressed Enter within a short timeout, without blocking forever.

```rust
use std::io::{self, Read};
use std::time::Duration;

#[cfg(unix)]
use std::os::unix::io::AsRawFd;

/// Wait for user to press Enter, with a timeout.
/// Returns true if Enter was pressed, false if timeout elapsed.
pub fn wait_for_enter(timeout: Duration) -> bool {
    // Platform-specific non-blocking input

    #[cfg(unix)]
    {
        use nix::sys::select::{select, FdSet};
        use nix::sys::time::TimeVal;
        use std::os::unix::io::AsRawFd;

        let stdin_fd = io::stdin().as_raw_fd();
        let mut read_fds = FdSet::new();
        read_fds.insert(stdin_fd);

        let mut tv = TimeVal::new(
            timeout.as_secs() as i64,
            timeout.subsec_micros() as i64,
        );

        match select(stdin_fd + 1, Some(&mut read_fds), None, None, Some(&mut tv)) {
            Ok(n) if n > 0 => {
                // Input available, consume it
                let mut buf = [0u8; 1];
                let _ = io::stdin().read(&mut buf);
                true
            }
            _ => false,
        }
    }

    #[cfg(windows)]
    {
        use std::sync::mpsc;
        use std::thread;

        let (tx, rx) = mpsc::channel();

        thread::spawn(move || {
            let mut buf = String::new();
            let _ = io::stdin().read_line(&mut buf);
            let _ = tx.send(());
        });

        rx.recv_timeout(timeout).is_ok()
    }
}
```

**Alternative (simpler) approach:** Use a crate like `crossterm` or `console` for cross-platform input handling. This avoids platform-specific code.

```toml
# Cargo.toml
[dependencies]
console = "0.15"  # or crossterm
```

```rust
use console::Term;
use std::time::Duration;

pub fn wait_for_enter_or_timeout(timeout: Duration) -> bool {
    let term = Term::stdout();

    // Check if stdin is a terminal (interactive)
    if !term.is_term() {
        return false; // Non-interactive, don't wait
    }

    // Use read_key with timeout
    match term.read_key_timeout(timeout) {
        Ok(Some(key)) => matches!(key, console::Key::Enter),
        _ => false,
    }
}
```

#### 2.3 Modify auth flow to offer browser opening

**File:** `src/main.rs` (in `run_auth` function)

```rust
async fn run_auth(force: bool) -> anyhow::Result<()> {
    // ... existing setup ...

    let response = manager.auth_client().initiate().await?;

    println!();
    println!("  To authenticate, visit:");
    println!();
    println!("    {}", response.verification_uri);
    println!();
    println!("  And enter this code: {}", response.user_code);
    println!();

    // NEW: Offer to open browser
    // Use verification_uri_complete if available (includes code pre-filled)
    let url_to_open = response.verification_uri_complete
        .as_deref()
        .unwrap_or(&response.verification_uri);

    print!("  Press Enter to open in browser (or wait to continue manually)... ");
    use std::io::Write;
    std::io::stdout().flush()?;

    // Wait up to 10 seconds for user to press Enter
    let should_open = wait_for_enter_or_timeout(Duration::from_secs(10));

    if should_open {
        match open_url(url_to_open) {
            Ok(true) => println!("  Browser opened!"),
            Ok(false) => println!("  Could not open browser. Please open the URL manually."),
            Err(e) => println!("  Failed to open browser: {e}"),
        }
    } else {
        println!(); // Clear the prompt line
    }

    println!();
    println!("  Waiting for authorization...");

    // ... rest of existing auth flow ...
}
```

#### 2.4 Update DeviceCodeResponse to include verification_uri_complete

**File:** `src/auth/device_flow.rs`

GitHub's device flow response includes `verification_uri_complete` which has the user code pre-filled:

```rust
#[derive(Debug, Deserialize)]
pub struct DeviceCodeResponse {
    pub device_code: String,
    pub user_code: String,
    pub verification_uri: String,
    #[serde(default)]
    pub verification_uri_complete: Option<String>,  // ADD THIS
    pub expires_in: u64,
    pub interval: u64,
}
```

**Completion Notes:**
- `open_url()` implemented cross-platform using `cmd /C start` (Windows), `open` (macOS), `xdg-open` (Linux) with no new crate dependencies
- `wait_for_enter_or_timeout()` uses `std::io::IsTerminal` (stable since Rust 1.70) for TTY detection and `thread + mpsc` for timed input wait — no platform-specific `select()` or crates needed
- Non-interactive / piped-stdin environments automatically skip the browser prompt
- `verification_uri_complete` added with `#[serde(default)]` so it degrades gracefully when absent
- Unit tests added: `tests/unit/browser_tests.rs`, `tests/unit/input_tests.rs`, `tests/unit/device_flow_tests.rs`

---

### Epic 3: Post-Start Guidance

**Status: DONE**

```rust
use std::env;

/// Display post-start guidance with environment variable setup instructions.
pub fn display_post_start_guidance(host: &str, port: u16) {
    let base_url = format!("http://{}:{}", host, port);

    println!();
    println!("╔══════════════════════════════════════════════════════════════════╗");
    println!("║                    Adapter Started Successfully                   ║");
    println!("╚══════════════════════════════════════════════════════════════════╝");
    println!();
    println!("Configure Claude Code to use this adapter:");
    println!();

    // Detect OS and shell
    #[cfg(target_os = "windows")]
    display_windows_guidance(&base_url);

    #[cfg(not(target_os = "windows"))]
    display_unix_guidance(&base_url);

    // Always show settings.json option
    display_settings_json_guidance(&base_url);

    println!();
}

#[cfg(target_os = "windows")]
fn display_windows_guidance(base_url: &str) {
    println!("  Option 1: PowerShell (current session)");
    println!("  ─────────────────────────────────────────");
    println!("    $env:ANTHROPIC_BASE_URL = \"{}\"", base_url);
    println!("    $env:ANTHROPIC_API_KEY = \"dummy\"");
    println!();

    println!("  Option 2: Command Prompt (current session)");
    println!("  ─────────────────────────────────────────────");
    println!("    set ANTHROPIC_BASE_URL={}", base_url);
    println!("    set ANTHROPIC_API_KEY=dummy");
    println!();

    println!("  To persist permanently:");
    println!("  ─────────────────────────────────────────────");
    println!("    1. Open: Settings > System > About > Advanced system settings");
    println!("    2. Click 'Environment Variables'");
    println!("    3. Add ANTHROPIC_BASE_URL = {}", base_url);
    println!("    4. Add ANTHROPIC_API_KEY = dummy");
    println!();
}

#[cfg(not(target_os = "windows"))]
fn display_unix_guidance(base_url: &str) {
    println!("  Option 1: Current session (bash/zsh)");
    println!("  ─────────────────────────────────────────");
    println!("    export ANTHROPIC_BASE_URL={}", base_url);
    println!("    export ANTHROPIC_API_KEY=dummy");
    println!();

    println!("  To persist permanently:");
    println!("  ─────────────────────────────────────────────");

    // Detect shell
    let shell = env::var("SHELL").unwrap_or_default();
    let rc_file = if shell.contains("zsh") {
        "~/.zshrc"
    } else {
        "~/.bashrc"
    };

    println!("    Add these lines to {}:", rc_file);
    println!();
    println!("      export ANTHROPIC_BASE_URL={}", base_url);
    println!("      export ANTHROPIC_API_KEY=dummy");
    println!();
}

fn display_settings_json_guidance(base_url: &str) {
    println!("  Option 3: Claude Code settings.json (recommended)");
    println!("  ─────────────────────────────────────────────────────");
    println!("    Create or edit ~/.claude/settings.json:");
    println!();
    println!("    {{");
    println!("      \"env\": {{");
    println!("        \"ANTHROPIC_BASE_URL\": \"{}\",", base_url);
    println!("        \"ANTHROPIC_API_KEY\": \"dummy\"");
    println!("      }}");
    println!("    }}");
    println!();
    println!("    Or for project-specific: <project>/.claude/settings.json");
    println!();
    println!("    Settings precedence (highest to lowest):");
    println!("      1. <project>/.claude/settings.local.json (gitignored)");
    println!("      2. <project>/.claude/settings.json");
    println!("      3. ~/.claude/settings.json");
}
```

#### 3.2 Integrate guidance into startup

**File:** `src/main.rs`

Add guidance display after successful server start (in foreground mode):

```rust
Command::Start { daemon, ... } => {
    // ... existing code ...

    if is_daemon {
        // Daemon mode: show brief message
        let pid = daemon::spawn_background(&args)?;
        println!("Adapter started in background (PID {pid})");
        println!();
        println!("Configure Claude Code:");
        println!("  export ANTHROPIC_BASE_URL=http://{}:{}", host, port);
        println!("  export ANTHROPIC_API_KEY=dummy");
        println!();
        println!("Or add to ~/.claude/settings.json (see README for details)");
        return Ok(());
    }

    // Foreground mode: show full guidance before blocking on server
    guidance::display_post_start_guidance(&host, port);

    server::run(&host, port, manager, true, config).await?;
}
```

#### 3.3 Add `--quiet` flag to suppress guidance

**File:** `src/cli.rs`

```rust
Start {
    // ... existing flags ...

    /// Suppress startup guidance messages
    #[arg(short, long)]
    quiet: bool,
}
```

---

### Epic 4: Documentation Updates

**Status: NOT STARTED**

#### 4.1 Update README.md

Add new sections and update existing ones:

**New "First-Time Setup" section:**

```markdown
## Quick Start

### 1. Install

```bash
cargo install --path .
```

### 2. Start the Adapter

```bash
copilot-adapter start
```

On first run, the adapter will:
1. Detect missing authentication and start the OAuth flow
2. Offer to open the GitHub authorization URL in your browser
3. Wait for you to authorize the application
4. Display configuration instructions for Claude Code

### 3. Configure Claude Code

Choose one of these methods:

**Method A: Environment Variables (session)**

[OS-specific examples as before]

**Method B: Claude Code Settings (recommended, persistent)**

Create or edit `~/.claude/settings.json`:

```json
{
  "env": {
    "ANTHROPIC_BASE_URL": "http://127.0.0.1:6767",
    "ANTHROPIC_API_KEY": "dummy"
  }
}
```

For project-specific configuration, create `.claude/settings.json` in your project root.

Settings precedence (highest to lowest):
1. `<project>/.claude/settings.local.json` (gitignored, for personal overrides)
2. `<project>/.claude/settings.json` (committed, for team sharing)
3. `~/.claude/settings.json` (user-level defaults)

### 4. Run Claude Code

```bash
claude
```
```

**Update Commands table:**

| Command | Description |
|---------|-------------|
| `copilot-adapter start` | Start adapter (auto-authenticates if needed) |
| `copilot-adapter start --daemon` | Start as background daemon (requires prior auth) |
| `copilot-adapter start --skip-auth` | Start without auto-authentication |
| `copilot-adapter start --quiet` | Start without displaying setup guidance |
| ... (rest unchanged) |

**Update Troubleshooting section:**

Add:

```markdown
### Auto-authentication not working in daemon mode

Daemon mode cannot perform interactive authentication. Run `copilot-adapter auth`
first, or start in foreground mode (`copilot-adapter start` without `--daemon`)
to authenticate interactively.

### Browser doesn't open during auth

The adapter waits 10 seconds for you to press Enter before opening the browser.
If your system doesn't support automatic browser opening, copy the URL manually.
On headless systems, the browser launch is skipped automatically.
```

---

## Testing Plan

### Manual Testing

1. **Fresh install (no credentials)**
   - Run `copilot-adapter start` → Should trigger auth flow
   - Run `copilot-adapter start --daemon` → Should fail with helpful message
   - Run `copilot-adapter start --skip-auth` → Should fail at first request

2. **Auth flow with browser**
   - Press Enter when prompted → Browser should open
   - Wait 10 seconds → Should continue without opening browser
   - Non-interactive (piped input) → Should skip browser prompt

3. **Post-start guidance**
   - Windows PowerShell → Should show PowerShell syntax
   - Windows CMD → Should show CMD syntax
   - Linux bash → Should show bash syntax and ~/.bashrc
   - Linux zsh → Should show zsh syntax and ~/.zshrc

4. **Settings.json integration**
   - Create `~/.claude/settings.json` with env block → Claude should use adapter
   - Create project `.claude/settings.json` → Should override user settings

### Unit Tests

- `browser::open_url()` — Mock or integration test per platform
- `input::wait_for_enter_or_timeout()` — Test timeout behavior
- `guidance::display_*` — Snapshot tests for output format

### Integration Tests

- Auth flow completion with mock GitHub OAuth server
- Server start with/without credentials
- Guidance output format validation

---

## Implementation Order

1. **Phase 1: Auto-auth** (Epic 1)
   - Add `--skip-auth` flag
   - Implement auth check before start
   - Handle daemon mode specially

2. **Phase 2: Browser launch** (Epic 2)
   - Add browser opening utility
   - Add input timeout utility
   - Update DeviceCodeResponse for verification_uri_complete
   - Modify auth flow with interactive prompt

3. **Phase 3: Guidance display** (Epic 3)
   - Create guidance module
   - Integrate into startup flow
   - Add `--quiet` flag

4. **Phase 4: Documentation** (Epic 4)
   - Update README.md
   - Add inline CLI help improvements

---

## Dependencies

### New crate dependencies (optional)

```toml
# For cross-platform terminal input (optional, can use std)
console = "0.15"  # or crossterm = "0.27"
```

### Existing dependencies (sufficient)

- `std::process::Command` for browser launching
- `std::io` for input handling
- Platform-specific APIs via `#[cfg(...)]`

---

## Risks and Mitigations

| Risk | Mitigation |
|------|------------|
| Browser launch fails on some systems | Graceful fallback to manual URL copy |
| Non-interactive environments hang on input | Timeout mechanism (10s default) |
| Settings.json format changes | Document minimum required structure |
| Daemon mode auth confusion | Clear error message directing to `auth` command |

---

## Success Criteria

1. New users can run `copilot-adapter start` and complete setup with zero prior knowledge
2. Auth flow works without manual URL copying (if user presses Enter)
3. Post-start output provides immediately usable copy-paste commands
4. Documentation clearly explains all configuration options including settings.json
5. Existing workflows (daemon mode, CI/CD) continue to work with `--skip-auth`