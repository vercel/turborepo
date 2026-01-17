# turborepo-task-hash

## Purpose

Computes cache keys for tasks based on their inputs. Determines when a task's cache should be invalidated by hashing all relevant inputs.

## Architecture

```
Task inputs
    ├── File contents (via daemon or SCM)
    ├── Environment variables
    ├── Task definition (from turbo.json)
    ├── Dependencies' hashes
    └── Global hash inputs
        │
        └── turborepo-hash
            └── Cache key
```

Key components:
- `TaskHasher` - Coordinates hash computation for tasks
- `GlobalHash` - Repository-wide inputs affecting all tasks
- Framework detection for automatic env var inclusion

## Notes

Uses the daemon for fast file hashing when available, falling back to SCM-based hashing. Framework detection (Next.js, Vite, etc.) automatically includes framework-specific environment variables in the hash.
