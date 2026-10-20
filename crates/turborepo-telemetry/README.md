# turborepo-telemetry

Handles anonymous telemetry for Turborepo, sending usage events to the Vercel API in the background with buffering and batching.

## Architecture

```
                                    ┌─────────────────────────────────────────┐
                                    │              Worker (tokio)             │
                                    │                                         │
┌──────────────┐    unbounded       │  ┌────────┐         ┌───────────────┐  │
│ telem(event) │ ──────────────────►│  │ Buffer │ ──────► │ TelemetryAPI  │  │
└──────────────┘    mpsc channel    │  └────────┘         └───────────────┘  │
                                    │                                         │
                                    │  Flush triggers:                        │
                                    │    - Buffer hits 10 events              │
                                    │    - 1 second timeout                   │
                                    │    - Shutdown signal                    │
                                    └─────────────────────────────────────────┘
```

**Key components:**

- `telem()` - Global function to send events. Safe to call from anywhere.
- `Worker` - Background tokio task that buffers events and flushes them in batches.
- `TelemetryConfig` - Persisted config at `~/.config/turborepo/telemetry.json` containing enabled state, anonymous ID, and private salt.

**Event types** (`events/`): `CommandEvent`, `RepoEvent`, `TaskEvent`, `GenericEvent`

## Notes

- **Telemetry is optional.** Users are shown a one-time notice. Disable via:
  - `turbo telemetry disable`
  - `TURBO_TELEMETRY_DISABLED=1`
  - `DO_NOT_TRACK=1`
- **All data is anonymized.** Sensitive fields (task names, package names) are one-way hashed with a per-machine salt that never leaves the machine.
- **Non-blocking.** Events are sent asynchronously; failures are silently logged.
- **Node port exists.** Changes here should be reflected in `packages/turbo-telemetry/src/config.ts`.

See https://turborepo.dev/docs/telemetry for full documentation.
