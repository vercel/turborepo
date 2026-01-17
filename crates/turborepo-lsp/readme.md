# turborepo-lsp

## Purpose

Language Server Protocol implementation for Turborepo. Provides IDE features for `turbo.json` files like completion, hover, and diagnostics.

## Architecture

```
turborepo-lsp
    └── tower-lsp server
        ├── Completions (task names, package names)
        ├── Hover information
        ├── Diagnostics (validation errors)
        └── Go to definition
```

Integrates with:
- Daemon for package discovery
- Repository analysis for package graph

## Notes

Designed for the `turbo-vsc` VS Code extension. Communicates via stdio. Uses the daemon when available for efficient package discovery.
