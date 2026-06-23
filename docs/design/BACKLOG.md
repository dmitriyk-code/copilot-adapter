# Copilot Adapter Backlog
Version: 0.5
Last updated: Jun 22 2026

## ToDo
Items that are yet to be fixed

(None — all items completed in [CONSOLIDATED.plan.md](./CONSOLIDATED.plan.md))

## Done
Items that are done

### Bugs
- 1M-context selection and Opus 4.7 effort no longer send non-existent Copilot
  model names. Copilot consolidated its Claude SKUs (no `-1m` / `-1m-internal` /
  `-high` / `-xhigh` model IDs), so the adapter stopped rewriting the model name
  for the `context-1m` header and routes Opus 4.7 effort via `reasoning.effort`.
  - **Design:** [COPILOT-1M-MODEL-CONSOLIDATION.design.md](./COPILOT-1M-MODEL-CONSOLIDATION.design.md), [COPILOT-1M-MODEL-CONSOLIDATION.plan.md](./COPILOT-1M-MODEL-CONSOLIDATION.plan.md)
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