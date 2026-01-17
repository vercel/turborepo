# turborepo-task-executor

## Purpose

Task execution infrastructure for Turborepo. Handles the actual running of tasks, including cache checking, process spawning, output handling, and cache saving.

## Architecture

```
turborepo-task-executor
    ├── TaskExecutor
    │   ├── Check cache (via turborepo-run-cache)
    │   ├── Spawn process (via turborepo-process)
    │   ├── Handle output (logs, UI)
    │   └── Save to cache on success
    └── Visitor pattern for task graph traversal
```

Uses trait abstractions for decoupling:
- `MfeConfigProvider` - Microfrontends configuration
- `HashTrackerProvider` - Hash tracking
- `TaskErrorCollector` / `TaskWarningCollector` - Error/warning collection

## Notes

Designed to be decoupled from `turborepo-lib` through traits. The executor doesn't know about the broader CLI context - it just runs individual tasks according to configuration.
