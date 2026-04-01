# Copilot Adapter Backlog
Version: 0.1
Last updated: Apr 1 2026

## ToDo
Items that are yet to be fixed

### Bugs
-- Authentication flow in --daemon mode does not work - just displays "Please run 'copilot-adapter auth' first, or use --skip-auth to bypass." - should lead through auth experience

### Nice-to-have items
- Save copilot-adapter running status (PID and port) under user's home directory (~/.copilot-adapter/status.json), not under temp directory,
  - Make sure to handle corner cases smoothly
- Save encrypted GitHub authentication token in a file under user's home directory, too (instead of Windows Credentials Manager),
- Allow running (and managing - start, stop, status) multiple instances of copilot-adapter:
  - Introduce a profile concept (a combination of port and github token - that can be identified by a profile name or by a port),
  - Profile concept is totally optional - by default, a predefined port 6767 and a predefined profile name can be used
  - Save profiles information under user's home directory

## Done
Items that are done

### Bugs
### Features