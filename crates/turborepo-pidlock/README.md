# turborepo-pidlock

## Purpose

PID file lock management for the daemon. Ensures only one daemon instance runs per repository.

## Architecture

```
turborepo-pidlock
    └── Pidlock
        ├── acquire() - Create lock file with PID
        ├── release() - Remove lock file
        └── get_owner() - Query current lock holder
```

## Notes

Vendored fork of the `pidlock` crate with Windows support and the ability to query the lock owner. Changes are pending upstream.
