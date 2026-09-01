# turbo daemon

Full docs: https://turborepo.dev/docs/reference/daemon

Manage the Turborepo background daemon (`turbod`).

```bash
turbo daemon [subcommand] [flags]
```

The daemon is **not** used for `turbo run` (deprecated). It is still used by `turbo watch` and the Turborepo LSP.

## Subcommands

| Subcommand | Description                                                                |
| ---------- | -------------------------------------------------------------------------- |
| `start`    | Ensure the daemon is running                                               |
| `stop`     | Stop the daemon                                                            |
| `restart`  | Restart the daemon                                                         |
| `status`   | Report daemon status (`--json` for machine-readable output)                |
| `clean`    | Stop the daemon and remove stale state (`--clean-logs=false` to keep logs) |
| `logs`     | Show daemon logs                                                           |

## Flags

- `--idle-time <duration>` — idle shutdown timeout (default: `4h0m0s`)
- `--turbo-json-path <path>` — custom `turbo.json` path to watch

## Examples

```bash
turbo daemon start
turbo daemon status --json
turbo daemon --idle-time=30m0s start
turbo daemon clean
```
