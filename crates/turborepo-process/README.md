# turborepo-process

Process management for running task commands. Spawns and manages child processes with support for PTY, signal forwarding, and graceful shutdown.

## Architecture

```
┌─────────────────────────────────────────────────────────────┐
│                     ProcessManager                          │
│  - Tracks all spawned children by TaskId                    │
│  - Coordinates shutdown (stop/wait)                         │
│  - Manages PTY size for terminal-attached processes         │
└─────────────────────────────────────────────────────────────┘
                              │
                              │ spawn()
                              ▼
┌─────────────────────────────────────────────────────────────┐
│                         Child                               │
│  - Wraps tokio::process::Child or portable_pty              │
│  - Handles graceful shutdown (SIGINT + timeout, then KILL)  │
│  - Pipes stdout/stderr to caller                            │
└─────────────────────────────────────────────────────────────┘
                              │
                              │ Command
                              ▼
┌─────────────────────────────────────────────────────────────┐
│                        Command                              │
│  - Builder for process configuration                        │
│  - Converts to tokio::process::Command or PTY command       │
└─────────────────────────────────────────────────────────────┘
```

## Key Types

- **`ProcessManager`**: Central coordinator. When open, spawns children; when closed, stops all children and rejects new spawns.
- **`Child`**: Handle to a running process. Supports `wait()`, `stop()`, `kill()`, and output piping.
- **`Command`**: Platform-agnostic command builder that works with both regular processes and PTY.
- **`ChildExit`**: Exit status enum (`Finished`, `Interrupted`, `Killed`, `KilledExternal`, `Failed`).

## Notes

- PTY support is inferred from terminal attachment on non-Windows platforms
- On Windows, graceful shutdown sends KILL immediately (no SIGINT equivalent)
- On Unix, processes are spawned in their own process group via `setsid()` to enable group signaling
- `stop_tasks()` allows selective process termination without closing the manager (used for watch mode restarts)
- Closing stdin on Windows with ConPTY immediately terminates the process, so stdin is kept open in that case
