# Usability Improvements for Copilot-Adapter

This document outlines suggested usability improvements to enhance the user experience of the copilot-adapter, focusing on authentication flow, configuration management, and integration with Claude Code.

## Executive Summary

The current implementation is functional but requires users to manually configure multiple systems. Key pain points:

1. **Manual environment variable setup** — Users must manually set `ANTHROPIC_BASE_URL` for Claude Code
2. **No persistent configuration file** — All settings are CLI flags, requiring repetition
3. **Silent daemon mode** — No feedback on daemon health after startup
4. **No "quick start" command** — Multiple steps required to get running
5. **No automatic Claude Code integration** — Users must know the right environment variable

---

## Improvement 1: Auto-Configure Claude Code Integration

### Problem

After starting the adapter, users must manually set:
```bash
export ANTHROPIC_BASE_URL=http://127.0.0.1:6767
```

This requires knowing:
- The correct environment variable name
- The correct URL format
- That they need to restart Claude Code after setting it

### Proposed Solution

Add a `copilot-adapter configure` command that:
1. Detects the user's shell (bash, zsh, fish, PowerShell)
2. Adds the environment variable to the appropriate config file
3. Optionally updates Claude Code's settings.json directly

**User Scenario (Before):**
```
1. copilot-adapter auth
2. copilot-adapter start --daemon
3. Google "how to set anthropic base url"
4. Manually edit .bashrc or .zshrc
5. Restart terminal
6. Restart Claude Code
```

**User Scenario (After):**
```
1. copilot-adapter auth
2. copilot-adapter configure --shell  # Auto-detects and configures
3. source ~/.bashrc                    # Or restart terminal
```

### Implementation Ideas

```rust
// src/cli.rs
#[derive(Subcommand)]
enum Commands {
    // ... existing
    /// Configure environment for Claude Code integration
    Configure {
        /// Configure shell environment (auto-detects shell)
        #[arg(long)]
        shell: bool,

        /// Path to shell config file (overrides auto-detection)
        #[arg(long)]
        shell_config: Option<PathBuf>,

        /// Port to configure (default: 6767)
        #[arg(short, long, default_value = "6767")]
        port: u16,

        /// Remove configuration
        #[arg(long)]
        uninstall: bool,
    },
}
```

**Detection logic:**
- Check `$SHELL` environment variable
- Look for `.bashrc`, `.zshrc`, `.config/fish/config.fish`, PowerShell profile
- Add/update `export ANTHROPIC_BASE_URL=http://127.0.0.1:PORT`
- Print clear instructions for activation

---

## Improvement 2: Configuration File Support

### Problem

Users must specify all options via CLI flags every time:
```bash
copilot-adapter start --daemon --port 9090 --log-level debug --models-cache-ttl 600
```

If they forget a flag, behavior changes unexpectedly.

### Proposed Solution

Add support for a configuration file (`~/.config/copilot-adapter/config.toml` or `~/.copilot-adapter.toml`):

```toml
# ~/.config/copilot-adapter/config.toml

[server]
port = 6767
bind_address = "127.0.0.1"

[logging]
level = "info"
# Optional: log_file = "/var/log/copilot-adapter.log"

[models]
cache_ttl = 300
static_models = false

[daemon]
auto_start = false  # Start daemon on login (future)
```

**Precedence:** CLI flags > Environment variables > Config file > Defaults

### Implementation Ideas

```rust
// src/config.rs
use serde::Deserialize;

#[derive(Deserialize, Default)]
pub struct Config {
    #[serde(default)]
    pub server: ServerConfig,
    #[serde(default)]
    pub logging: LoggingConfig,
    #[serde(default)]
    pub models: ModelsConfig,
}

impl Config {
    pub fn load() -> Self {
        let paths = [
            dirs::config_dir().map(|p| p.join("copilot-adapter/config.toml")),
            dirs::home_dir().map(|p| p.join(".copilot-adapter.toml")),
        ];

        for path in paths.into_iter().flatten() {
            if path.exists() {
                if let Ok(content) = std::fs::read_to_string(&path) {
                    if let Ok(config) = toml::from_str(&content) {
                        return config;
                    }
                }
            }
        }
        Self::default()
    }
}
```

Add `copilot-adapter config` subcommand:
- `copilot-adapter config init` — Create default config file with comments
- `copilot-adapter config show` — Display effective configuration (merged)
- `copilot-adapter config path` — Print config file location

---

## Improvement 3: Quick Start / Setup Wizard

### Problem

New users face a multi-step process:
1. Download/install binary
2. Run `auth`
3. Run `start`
4. Configure environment
5. Test it works

Each step can fail, and there's no guided experience.

### Proposed Solution

Add a `copilot-adapter setup` command that runs an interactive wizard:

```
$ copilot-adapter setup

╔══════════════════════════════════════════════════╗
║         Copilot Adapter Setup Wizard             ║
╚══════════════════════════════════════════════════╝

Step 1/4: GitHub Authentication
────────────────────────────────
Please visit: https://github.com/login/device
Enter code: ABCD-1234

Waiting for authorization... ✓ Authenticated as @username

Step 2/4: Server Configuration
──────────────────────────────
Port [6767]:
Log level (error/warn/info/debug/trace) [info]:

Step 3/4: Shell Integration
───────────────────────────
Detected shell: zsh
Add ANTHROPIC_BASE_URL to ~/.zshrc? [Y/n]:

Added to ~/.zshrc:
  export ANTHROPIC_BASE_URL=http://127.0.0.1:6767

Step 4/4: Start Adapter
───────────────────────
Start as daemon now? [Y/n]:

✓ Daemon started (PID: 12345)

════════════════════════════════════════════════════
Setup complete! Next steps:
  1. Run: source ~/.zshrc  (or restart your terminal)
  2. Start Claude Code
  3. Your requests will be routed through GitHub Copilot
════════════════════════════════════════════════════
```

### Implementation Ideas

```rust
// src/commands/setup.rs
use dialoguer::{Confirm, Input, Select};

pub async fn run_setup_wizard() -> Result<()> {
    println_header("Copilot Adapter Setup Wizard");

    // Step 1: Auth
    if !has_valid_token().await {
        println_step(1, 4, "GitHub Authentication");
        run_device_flow().await?;
    } else {
        println!("✓ Already authenticated");
    }

    // Step 2: Configuration
    println_step(2, 4, "Server Configuration");
    let port: u16 = Input::new()
        .with_prompt("Port")
        .default(6767)
        .interact()?;

    // ... etc
}
```

Dependencies: `dialoguer` for interactive prompts, `indicatif` for progress bars.

---

## Improvement 4: Status Command Enhancements

### Problem

Current `status` command only shows if daemon is running:
```
Adapter is running (PID: 12345)
```

Users don't know:
- Is authentication valid?
- What port is it running on?
- Is it healthy? Can it reach Copilot API?
- What configuration is active?

### Proposed Solution

Enhanced status output:

```
$ copilot-adapter status

╭─────────────────────────────────────────────────╮
│ Copilot Adapter Status                          │
├─────────────────────────────────────────────────┤
│ Daemon:     Running (PID: 12345)                │
│ Uptime:     2h 34m                              │
│ Port:       6767                                │
│ Endpoint:   http://127.0.0.1:6767               │
├─────────────────────────────────────────────────┤
│ Authentication:                                 │
│   GitHub:   ✓ Authenticated as @username        │
│   Token:    Valid (expires in 25m)              │
├─────────────────────────────────────────────────┤
│ Health:                                         │
│   Local:    ✓ Responding                        │
│   Copilot:  ✓ API reachable                     │
├─────────────────────────────────────────────────┤
│ Environment:                                    │
│   ANTHROPIC_BASE_URL: ✓ Set correctly           │
╰─────────────────────────────────────────────────╯
```

Add flags:
- `--json` — Machine-readable output
- `--health` — Only show health check result (for scripts)

### Implementation Ideas

```rust
// src/commands/status.rs
pub struct StatusInfo {
    pub daemon: DaemonStatus,
    pub auth: AuthStatus,
    pub health: HealthStatus,
    pub environment: EnvStatus,
}

impl StatusInfo {
    pub async fn gather(port: u16) -> Self {
        let (daemon, auth, health, env) = tokio::join!(
            check_daemon(),
            check_auth(),
            check_health(port),
            check_environment(port),
        );
        Self { daemon, auth, health, environment: env }
    }
}
```

---

## Improvement 5: Automatic Browser Launch for Auth

### Problem

Current auth flow prints a URL and code:
```
Please visit: https://github.com/login/device
Enter code: ABCD-1234
```

User must manually copy the URL, open browser, paste URL, then enter code.

### Proposed Solution

Automatically open the browser with the full verification URL:
```
Opening browser for GitHub authentication...
If browser doesn't open, visit: https://github.com/login/device
Enter code: ABCD-1234

Waiting for authorization...
```

GitHub's device flow supports a `verification_uri_complete` that includes the code in the URL, providing one-click authorization.

### Implementation Ideas

```rust
// src/auth/device_flow.rs
use webbrowser;

pub async fn authenticate() -> Result<Token> {
    let device_code = request_device_code().await?;

    // Try to open browser with complete URL
    let browser_opened = if let Some(complete_uri) = &device_code.verification_uri_complete {
        webbrowser::open(complete_uri).is_ok()
    } else {
        webbrowser::open(&device_code.verification_uri).is_ok()
    };

    if browser_opened {
        println!("Opening browser for GitHub authentication...");
        println!("If browser doesn't open, visit: {}", device_code.verification_uri);
    } else {
        println!("Please visit: {}", device_code.verification_uri);
    }

    if device_code.verification_uri_complete.is_none() {
        println!("Enter code: {}", device_code.user_code);
    }

    // ... polling loop
}
```

Add `--no-browser` flag for headless/SSH environments.

---

## Improvement 6: Connection Diagnostics

### Problem

When things don't work, users have no diagnostic tools:
- Is the adapter running?
- Is the token valid?
- Can it reach Copilot API?
- Is the request format correct?

### Proposed Solution

Add `copilot-adapter diagnose` command:

```
$ copilot-adapter diagnose

Running diagnostics...

[1/6] Checking daemon status...
      ✓ Daemon is running (PID: 12345)

[2/6] Checking port availability...
      ✓ Port 6767 is responding

[3/6] Checking authentication...
      ✓ GitHub token valid (expires in 25m)

[4/6] Checking Copilot token...
      ✓ Copilot token valid (expires in 28m)

[5/6] Testing Copilot API connectivity...
      ✓ Successfully reached api.githubcopilot.com
      ✓ Models endpoint returned 12 models

[6/6] Testing end-to-end request...
      ✓ Test message processed successfully

════════════════════════════════════════
All checks passed! The adapter is working correctly.

To test with Claude Code:
  1. Ensure ANTHROPIC_BASE_URL=http://127.0.0.1:6767
  2. Run: claude "hello"
════════════════════════════════════════
```

On failure, show specific remediation steps:

```
[4/6] Checking Copilot token...
      ✗ Token refresh failed: 403 Forbidden

Diagnosis: Your GitHub Copilot subscription may have expired
           or you may not have access to the Copilot API.

Remediation:
  1. Check your subscription at: https://github.com/settings/copilot
  2. Ensure you have GitHub Copilot Individual or Business
  3. Try re-authenticating: copilot-adapter auth --force

For more help: https://github.com/your-repo/issues
```

### Implementation Ideas

```rust
// src/commands/diagnose.rs
pub async fn run_diagnostics() -> Result<DiagnosticReport> {
    let checks = vec![
        ("Daemon status", check_daemon),
        ("Port availability", check_port),
        ("GitHub auth", check_github_auth),
        ("Copilot token", check_copilot_token),
        ("API connectivity", check_api_connectivity),
        ("End-to-end test", check_e2e),
    ];

    let mut results = Vec::new();
    for (i, (name, check)) in checks.iter().enumerate() {
        print!("[{}/{}] {}...", i + 1, checks.len(), name);
        match check().await {
            Ok(info) => {
                println!(" ✓ {}", info);
                results.push(CheckResult::Pass(info));
            }
            Err(e) => {
                println!(" ✗ {}", e.message);
                if let Some(remediation) = e.remediation {
                    println!("\n{}", remediation);
                }
                results.push(CheckResult::Fail(e));
            }
        }
    }
    // ...
}
```

---

## Improvement 7: Systemd/launchd Service Installation

### Problem

`--daemon` mode works but:
- Doesn't survive system reboots
- No automatic restart on crash
- Users must manually start it each session

### Proposed Solution

Add `copilot-adapter service` commands for OS-native service management:

```bash
# Install as system service
copilot-adapter service install

# Check service status
copilot-adapter service status

# View service logs
copilot-adapter service logs

# Uninstall service
copilot-adapter service uninstall
```

**Linux (systemd):**
```ini
# ~/.config/systemd/user/copilot-adapter.service
[Unit]
Description=Copilot Adapter for Claude Code
After=network.target

[Service]
Type=simple
ExecStart=/usr/local/bin/copilot-adapter start
Restart=on-failure
RestartSec=5

[Install]
WantedBy=default.target
```

**macOS (launchd):**
```xml
<!-- ~/Library/LaunchAgents/com.github.copilot-adapter.plist -->
<plist version="1.0">
<dict>
    <key>Label</key>
    <string>com.github.copilot-adapter</string>
    <key>ProgramArguments</key>
    <array>
        <string>/usr/local/bin/copilot-adapter</string>
        <string>start</string>
    </array>
    <key>RunAtLoad</key>
    <true/>
    <key>KeepAlive</key>
    <true/>
</dict>
</plist>
```

**Windows (Task Scheduler or Windows Service):**
```powershell
# Create scheduled task to run at login
schtasks /create /tn "CopilotAdapter" /tr "copilot-adapter start" /sc onlogon
```

### Implementation Ideas

```rust
// src/commands/service.rs
pub enum ServiceCommand {
    Install,
    Uninstall,
    Status,
    Logs,
}

#[cfg(target_os = "linux")]
mod linux {
    pub fn install_systemd_service(config: &Config) -> Result<()> {
        let unit = generate_unit_file(config);
        let path = dirs::config_dir()
            .unwrap()
            .join("systemd/user/copilot-adapter.service");
        std::fs::write(&path, unit)?;

        Command::new("systemctl")
            .args(["--user", "daemon-reload"])
            .status()?;
        Command::new("systemctl")
            .args(["--user", "enable", "--now", "copilot-adapter"])
            .status()?;
        Ok(())
    }
}
```

---

## Improvement 8: Startup Validation

### Problem

`copilot-adapter start` succeeds even if:
- No valid GitHub token exists
- Token refresh fails
- Copilot API is unreachable

User discovers the problem only when Claude Code fails.

### Proposed Solution

Add pre-flight checks before server starts:

```
$ copilot-adapter start

Pre-flight checks:
  ✓ GitHub token found
  ✓ Token refresh successful
  ✓ Copilot API reachable

Starting server on http://127.0.0.1:6767...
```

If checks fail:
```
$ copilot-adapter start

Pre-flight checks:
  ✗ No GitHub token found

Error: Not authenticated. Run 'copilot-adapter auth' first.
```

Add `--skip-preflight` flag to bypass for advanced users.

### Implementation Ideas

```rust
// src/server.rs
pub async fn run_server(args: &StartArgs) -> Result<()> {
    if !args.skip_preflight {
        run_preflight_checks().await?;
    }

    // ... existing server startup
}

async fn run_preflight_checks() -> Result<()> {
    println!("Pre-flight checks:");

    // Check 1: Token exists
    let token = match storage::get_github_token() {
        Some(t) => {
            println!("  ✓ GitHub token found");
            t
        }
        None => {
            println!("  ✗ No GitHub token found");
            return Err(Error::NotAuthenticated);
        }
    };

    // Check 2: Token refresh works
    match refresh_copilot_token(&token).await {
        Ok(_) => println!("  ✓ Token refresh successful"),
        Err(e) => {
            println!("  ✗ Token refresh failed: {}", e);
            return Err(e);
        }
    }

    // Check 3: API reachable (optional, can be slow)
    // ...

    Ok(())
}
```

---

## Improvement 9: Better Error Messages

### Problem

Current errors are technical and don't guide users:
```
Error: reqwest::Error { kind: Status(403), ... }
```

### Proposed Solution

User-friendly error messages with context and remediation:

```
Error: GitHub Copilot API returned 403 Forbidden

This usually means:
  • Your GitHub Copilot subscription has expired
  • You don't have access to the Copilot API
  • Your organization has restricted Copilot access

To fix:
  1. Check your subscription: https://github.com/settings/copilot
  2. If subscription is active, try: copilot-adapter auth --force
  3. If using an organization account, contact your admin

For more help: copilot-adapter diagnose
```

### Implementation Ideas

```rust
// src/error.rs
#[derive(Debug)]
pub struct UserError {
    pub message: String,
    pub context: Option<String>,
    pub remediation: Vec<String>,
    pub help_command: Option<String>,
}

impl From<CopilotApiError> for UserError {
    fn from(e: CopilotApiError) -> Self {
        match e.status {
            StatusCode::FORBIDDEN => UserError {
                message: "GitHub Copilot API returned 403 Forbidden".into(),
                context: Some("This usually means subscription or access issues".into()),
                remediation: vec![
                    "Check your subscription: https://github.com/settings/copilot".into(),
                    "Try re-authenticating: copilot-adapter auth --force".into(),
                ],
                help_command: Some("copilot-adapter diagnose".into()),
            },
            // ... other status codes
        }
    }
}
```

---

## Improvement 10: Verbose/Quiet Output Modes

### Problem

Output verbosity is either info level (minimal) or debug/trace (overwhelming). No middle ground for users who want progress updates without log spam.

### Proposed Solution

Add `--verbose` and `--quiet` flags orthogonal to log level:

```bash
# Quiet: only errors
copilot-adapter start --quiet

# Normal (default): key events
copilot-adapter start

# Verbose: progress + explanations
copilot-adapter start --verbose
```

**Normal output:**
```
Starting server on http://127.0.0.1:6767...
```

**Verbose output:**
```
Starting Copilot Adapter v0.1.0

Configuration:
  Port: 6767
  Log level: info
  Models cache TTL: 300s

Initializing token manager...
  Loaded GitHub token from keyring
  Refreshing Copilot token... done (expires in 30m)

Starting HTTP server...
  Binding to 127.0.0.1:6767
  Routes: /health, /v1/messages, /v1/models

Server ready! Claude Code can now connect.
Press Ctrl+C to stop.
```

---

## Priority Ranking

| Priority | Improvement | Impact | Effort |
|----------|-------------|--------|--------|
| 1 | Quick Start Wizard (#3) | High | Medium |
| 2 | Auto-Configure Claude Code (#1) | High | Low |
| 3 | Startup Validation (#8) | High | Low |
| 4 | Enhanced Status (#4) | Medium | Low |
| 5 | Better Error Messages (#9) | Medium | Medium |
| 6 | Configuration File (#2) | Medium | Medium |
| 7 | Connection Diagnostics (#6) | Medium | Medium |
| 8 | Auto Browser Launch (#5) | Low | Low |
| 9 | Service Installation (#7) | Low | High |
| 10 | Verbose/Quiet Modes (#10) | Low | Low |

---

## Implementation Roadmap

### Phase 1: Quick Wins (1-2 days)
- Startup validation (#8)
- Auto browser launch (#5)
- Enhanced status command (#4)

### Phase 2: Core UX (3-5 days)
- Auto-configure Claude Code (#1)
- Better error messages (#9)
- Verbose/quiet modes (#10)

### Phase 3: Polish (1 week)
- Quick start wizard (#3)
- Configuration file support (#2)
- Connection diagnostics (#6)

### Phase 4: Advanced (future)
- Service installation (#7)

---

## Dependencies to Add

```toml
# Cargo.toml additions
[dependencies]
dialoguer = "0.11"      # Interactive prompts
indicatif = "0.17"      # Progress bars
webbrowser = "0.8"      # Open browser
toml = "0.8"            # Config file parsing
dirs = "5.0"            # Standard directories
```

---

## Open Questions

1. **Should `setup` be the default command?** Running just `copilot-adapter` could launch the wizard if not configured.

2. **Config file location:** `~/.config/copilot-adapter/config.toml` vs `~/.copilot-adapter.toml`?

3. **Service installation scope:** User service (recommended) or system service?

4. **Claude Code settings.json:** Should we directly modify it, or only environment variables?

5. **Telemetry:** Should we add opt-in anonymous usage stats to understand pain points?