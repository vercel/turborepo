# turborepo-shim

## Purpose

Handles version resolution and delegation for the `turbo` binary. Finds and invokes the correct version of turbo based on the repository's local installation, enabling per-repo version pinning.

## Architecture

```
User runs `turbo`
    └── Shim logic
        ├── Check for local turbo in node_modules
        │   └── If found: spawn local turbo as child process
        └── If not found: run current binary directly
```

Uses trait-based dependency injection (`TurboRunner`) to avoid circular dependencies with `turborepo-lib`.

Key components:
- `ShimArgs` - Parsed arguments needed for shim decisions
- `TurboState` - Tracks which turbo binary should execute
- `run()` - Main entry point for shim logic

## Notes

The shim ensures that running `turbo` always uses the version specified in a project's `package.json`, even if a different global version is installed. This is critical for reproducible builds across team members.
