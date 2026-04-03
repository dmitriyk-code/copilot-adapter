# Copilot Adapter Backlog
Version: 0.4
Last updated: Apr 2 2026

## ToDo
Items that are yet to be fixed

(None — all items completed in [CONSOLIDATED.plan.md](./CONSOLIDATED.plan.md))

## Done
Items that are done

### Bugs
- Authentication flow in --daemon mode now leads through auth experience (same as foreground)
  - **Design:** [DAEMON-AUTH.design.md](./DAEMON-AUTH.design.md)

### Features
- Runtime status stored in ~/.copilot-adapter/status.json (with legacy fallback)
  - **Design:** [HOME-DIR-STATUS.design.md](./HOME-DIR-STATUS.design.md)
- Credentials stored in `~/.copilot-adapter/profiles/<name>/github-copilot.json` using platform-native encryption (DPAPI on Windows; OS keyring on macOS/Linux); automatic migration from old XOR-obfuscated `credentials.json`
  - **Design:** [HOME-DIR-TOKEN.design.md](./HOME-DIR-TOKEN.design.md), [NATIVE-CREDENTIAL-STORAGE.plan.md](./NATIVE-CREDENTIAL-STORAGE.plan.md)
- Multi-instance profiles via --profile / -P flag; profiles subcommand for management
  - **Design:** [MULTI-INSTANCE-PROFILES.design.md](./MULTI-INSTANCE-PROFILES.design.md)

**Implementation:** [CONSOLIDATED.plan.md](./CONSOLIDATED.plan.md) — all four items above were implemented in a single, sequenced plan (10 epics).