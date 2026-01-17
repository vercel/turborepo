# turborepo-types

## Purpose

Shared types used across the Turborepo crate ecosystem. Provides foundational types and traits to avoid circular dependencies between higher-level crates.

## Architecture

```
turborepo-types (foundation layer)
    │
    ├── Used by: turborepo-engine, turborepo-task-hash, etc.
    └── Provides:
        ├── TaskDefinition - Task configuration
        ├── EnvMode - strict/loose environment handling
        ├── OutputLogsMode - log output control
        ├── UIMode - terminal UI selection
        └── Traits for cross-crate abstraction
```

Key traits:
- `EngineInfo` - Access to task definitions and dependencies
- `RunOptsInfo` - Access to run options
- `HashTrackerInfo` - Access to task hash information

## Notes

This crate exists to break dependency cycles. Types that need to be shared across multiple crates without creating circular dependencies live here. It's a foundation layer with minimal dependencies of its own.
