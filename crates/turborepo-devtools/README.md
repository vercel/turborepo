# turborepo-devtools

## Purpose

WebSocket-based devtools server for visualizing package and task graphs in real-time. Supports live updates as the repository changes.

## Architecture

```
turborepo-devtools
    ├── WebSocket server (default port 9876)
    ├── Graph serialization
    └── File watcher integration
        └── Push updates to connected clients
```

Provides real-time views of:
- Package dependency graph
- Task dependency graph
- Changes as files are modified

## Notes

Designed for integration with visualization tools. The server pushes updates when repository structure changes are detected via file watching.
