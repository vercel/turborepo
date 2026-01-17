# turborepo-run-cache

## Purpose

Task-aware caching layer that wraps `turborepo-cache` with task-specific semantics. Handles log files, output modes, and integration with the daemon for output tracking.

## Architecture

```
turborepo-run-cache
    ├── RunCache (per-run cache state)
    │   └── TaskCache (per-task operations)
    │       ├── Check cache for task hash
    │       ├── Restore outputs on hit
    │       ├── Handle log file replay
    │       └── Save outputs on completion
    └── turborepo-cache (underlying storage)
```

Key responsibilities:
- Log file handling and output mode management
- Integration with daemon for file watching
- Task definition-aware output glob handling
- Cache hit/miss telemetry

## Notes

This crate bridges the gap between raw cache storage (`turborepo-cache`) and the task execution system. It understands task outputs, logs, and how to properly restore cached state.
