# turborepo-lib

## Purpose

The core library containing all Turborepo CLI logic. Handles command parsing, task execution, caching, and orchestration of the entire `turbo` experience.

## Architecture

This is the central hub that ties together most other crates:

```
turborepo-lib
    ├── CLI parsing and command dispatch
    ├── Run orchestration (turbo run)
    │   ├── turborepo-engine (task graph)
    │   ├── turborepo-task-executor (execution)
    │   ├── turborepo-task-hash (cache keys)
    │   └── turborepo-run-cache (cache operations)
    ├── Repository analysis
    │   ├── turborepo-repository (package graph)
    │   ├── turborepo-lockfiles (dependency analysis)
    │   └── turborepo-scm (git integration)
    ├── Configuration
    │   ├── turborepo-config (merged config)
    │   └── turborepo-turbo-json (parsing)
    └── Supporting systems
        ├── turborepo-daemon (file watching)
        ├── turborepo-cache (local + remote)
        ├── turborepo-ui (terminal output)
        └── turborepo-telemetry (anonymous usage)
```

Key modules:
- `cli/` - Command-line argument parsing
- `commands/` - Implementation of each CLI command
- `run/` - The `turbo run` command orchestration
- `query/` - GraphQL query interface

## Notes

Only consumed by the `turborepo` binary crate. External consumers should use more specific crates like `turborepo-repository` for the package graph.
