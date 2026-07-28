# turborepo-globwatch

## Purpose

Watches file system changes filtered by glob patterns. Wraps `notify` with glob-based filtering and provides "flushing" semantics to ensure watchers are synchronized with the file system.

## Architecture

```
File system events (notify)
    └── turborepo-globwatch
        ├── Filter events by glob patterns
        ├── Flush mechanism (cookie files)
        └── Stream of matching events
```

Exposed as a `Stream` and `Sink`:
- Stream produces `notify` events for matching files
- Sink allows updating watch configuration on-the-fly

Optimizes by watching the minimum set of directories needed to cover the glob patterns.

## Notes

On some filesystems, events may arrive out of order or delayed. The flush mechanism uses cookie files to provide a round-trip guarantee, ensuring the watcher is current before answering queries. Used by the daemon for reliable file change detection.
