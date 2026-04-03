# Credential Storage Migration Guide

## Overview

Starting with the native credential storage release, copilot-adapter uses
platform-native encryption for credential storage. This replaces the previous
XOR obfuscation system with always-on OS-native security.

## What Changed

### Before (v1 format)
- File: `credentials.json` (binary, XOR-obfuscated)
- Security: Reversible obfuscation (not cryptographic)
- Opt-in: `--use-keyring` flag for real encryption

### After (v2 format)
- File: `github-copilot.json` (human-readable JSON)
- Security: Platform-native encryption (always on)
  - Windows: DPAPI (`CryptProtectData` / `CryptUnprotectData`)
  - macOS: Keychain (via `keyring` crate)
  - Linux: Secret Service (via `keyring` crate)
- No flags needed — encryption is automatic

## Automatic Migration

The adapter automatically migrates your credentials when you first run the
updated version:

1. Old `credentials.json` is read (best-effort XOR decryption)
2. Token is re-encrypted and stored in new `github-copilot.json` format
3. Old `credentials.json` is deleted

**Important:** Migration prioritizes security over preservation. If the
migration fails (corrupted file, username changed, etc.), you'll need to
re-authenticate. The old insecure file is **always** deleted regardless of
whether the token was successfully recovered.

## Manual Migration (if needed)

If automatic migration fails:

```bash
copilot-adapter logout  # Clear any partial state
copilot-adapter auth    # Re-authenticate
```

For a specific profile:

```bash
copilot-adapter logout -P myprofile
copilot-adapter auth -P myprofile
```

## Troubleshooting

### "No secure credential storage available"

On Linux, this means no Secret Service provider is running. Install one:

- **Ubuntu/Debian:** `sudo apt install gnome-keyring` or `sudo apt install kde-wallet`
- **Arch:** `sudo pacman -S gnome-keyring` or `sudo pacman -S kwalletmanager`
- **Fedora:** Usually pre-installed with GNOME or KDE

Then start the service and re-run `copilot-adapter auth`.

### "Failed to read old XOR credentials"

This can happen if:
- Your OS username changed since the credentials were stored
- The old credentials file is corrupted
- File permissions changed

**Solution:** Re-authenticate with `copilot-adapter auth`. The old insecure
file will be cleaned up automatically.

### Credentials work in foreground but not as daemon

Ensure you authenticated in the same user context that the daemon runs under.
DPAPI (Windows) and keyring (macOS/Linux) are tied to the current user session.

## File Format Reference

### Windows (DPAPI)

```json
{
  "version": 2,
  "storage": "dpapi",
  "github_token": "AQAAANCM...base64..."
}
```

The `github_token` field contains the DPAPI-encrypted token encoded as base64.
It can only be decrypted by the same Windows user account that encrypted it.

### macOS/Linux (Keyring)

```json
{
  "version": 2,
  "storage": "keyring"
}
```

The token is stored in the OS keyring (not in the file). The keyring entry uses:
- **Service:** `copilot-adapter`
- **Username:** `{profile}:github_token` (e.g., `default:github_token`)

### File Location

Credentials are stored per-profile at:

```
~/.copilot-adapter/profiles/<name>/github-copilot.json
```

The default profile name is `default`, so the default path is:

```
~/.copilot-adapter/profiles/default/github-copilot.json
```

## Removed Features

The `--use-keyring` flag has been removed from both `auth` and `start` commands.
Platform-native encryption is now always enabled — there is no opt-out. If you
were previously using `--use-keyring`, no action is needed; credentials stored
in the OS keyring will continue to work.
