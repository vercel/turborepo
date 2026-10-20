# turborepo-cli

## Purpose

The command-line interface crate for Turborepo. It owns argument parsing, command dispatch, shim adapters, panic handling, and CLI integration with focused runtime crates.

## Architecture

This crate is the CLI-facing composition layer:

```
turborepo-cli
    ├── CLI parsing and command dispatch
    ├── Runtime orchestration
    │   ├── turborepo-run
    │   └── turborepo-watch
    ├── Configuration
    │   ├── turborepo-config
    │   └── turborepo-turbo-json
    └── CLI integrations
        ├── turborepo-daemon
        ├── turborepo-ui
        └── turborepo-telemetry
```

Key modules:
- `cli/` - Command-line argument parsing
- `commands/` - Implementation of each CLI command
- `shim.rs` - Adapters between the shim and CLI commands
- `devtools.rs` - CLI wrapper for the devtools server

## Notes

Only consumed by the `turborepo` binary crate. External consumers should use more specific crates like `turborepo-repository` for the package graph.
