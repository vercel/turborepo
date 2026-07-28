# turborepo-daemon

## Purpose

Background daemon that watches files and pre-computes data to speed up turbo execution. Each repository has its own daemon instance.

## Architecture

```
turborepo-daemon
    ├── gRPC server (tonic)
    │   ├── File hash queries
    │   ├── Package change notifications
    │   └── Output tracking
    ├── FileWatching
    │   ├── FileSystemWatcher (notify)
    │   ├── GlobWatcher - glob-filtered events
    │   └── PackageWatcher - package change detection
    └── Cookie files for event synchronization
```

Communication:
- `DaemonConnector` - Connects to running daemon
- `DaemonClient` - gRPC client interface

## Notes

Cookie files ensure proper event ordering - we don't want stale file system events during queries. The daemon significantly speeds up file hashing by maintaining a persistent view of the file system rather than rescanning on each run.
