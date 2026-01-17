# turborepo-filewatch

## Purpose

File watching utilities with cross-platform consistency. Provides watchers that consume file events and produce derived data like changed packages.

## Architecture

```
turborepo-filewatch
    ├── FileSystemWatcher
    │   ├── macOS: FSEvents (custom impl)
    │   ├── Linux: inotify (recursive)
    │   └── Windows: ReadDirectoryChanges
    ├── PackageWatcher - maps file changes to packages
    └── GlobWatcher - filters by glob patterns
```

Platform differences:
- macOS: No recursive watch, no ancestor watching
- Linux: Recursive watch, watches ancestors
- Windows: No recursive watch, watches ancestors

## Notes

Event processing must be fast to avoid lag. The common pattern is: accumulate events in one thread, process in another. Uses `tokio::sync::Notify` or intervals to coordinate. A `Lagged` event indicates the receiver fell behind.
