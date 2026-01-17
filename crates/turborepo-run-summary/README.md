# turborepo-run-summary

Tracks task execution during `turbo run` and generates run summaries for reporting, dry-run output, and persistence.

## Architecture

```
┌─────────────────────────────────────────────────────────────────────┐
│                           RunTracker                                │
│  Created at run start, tracks SCM state and spawns ExecutionTracker │
└──────────────────────────────┬──────────────────────────────────────┘
                               │
                               │ spawns per-task
                               ▼
┌─────────────────────────────────────────────────────────────────────┐
│                          TaskTracker                                │
│  Tracks individual task lifecycle: start → cached/succeeded/failed  │
│  Sends events via channel to ExecutionTracker                       │
└──────────────────────────────┬──────────────────────────────────────┘
                               │
                               │ events aggregated
                               ▼
┌─────────────────────────────────────────────────────────────────────┐
│                       ExecutionTracker                              │
│  Background thread collecting task events into SummaryState         │
└──────────────────────────────┬──────────────────────────────────────┘
                               │
                               │ at run end
                               ▼
┌─────────────────────────────────────────────────────────────────────┐
│                          RunSummary                                 │
│  Final summary combining:                                           │
│  - GlobalHashSummary (env vars, files, dependencies)                │
│  - ExecutionSummary (timing, success/fail/cache counts)             │
│  - Vec<TaskSummary> (per-task hash, inputs, outputs, cache status)  │
└─────────────────────────────────────────────────────────────────────┘
                               │
                               ▼
              ┌────────────────┴────────────────┐
              │                                 │
         JSON file                        Terminal output
   (.turbo/runs/<id>.json)             (--dry or run complete)
```

**Key types:**

- `RunTracker` - Live tracker created before task execution begins
- `ExecutionTracker` / `TaskTracker` - Channel-based event collection for concurrent tasks
- `RunSummary` - Serializable final summary (saved to `.turbo/runs/`)
- `TaskSummaryFactory` - Constructs `TaskSummary` from engine/hash tracker data
- `GlobalHashSummary` - Global cache inputs (root files, env vars, external deps)

## Notes

- Traits like `EngineInfo`, `HashTrackerInfo`, and `RunOptsInfo` are re-exported from `turborepo-types`. This enables proper dependency direction: infrastructure crates (`turborepo-engine`, `turborepo-task-hash`) implement these traits without depending on this crate.
- `SinglePackageRunSummary` is a separate struct for single-package repos, omitting the `packages` field and simplifying task IDs.
- The `TaskTracker` uses a typestate pattern: `TaskTracker<()>` (not started) → `TaskTracker<DateTime<Local>>` (started).
