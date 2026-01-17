# turborepo-boundaries

## Purpose

Enforces architectural boundaries between packages. Validates that import relationships respect configured rules about which packages can depend on which others.

## Architecture

```
turborepo-boundaries
    ├── config/ - Boundary rules from turbo.json
    ├── tags/ - Package tag system for grouping
    ├── imports/ - Import location tracking
    └── Validation engine
        ├── Parse source files (swc)
        ├── Trace imports (turbo-trace)
        └── Check against rules
```

Rules can specify:
- Allowed/denied dependencies between tagged packages
- Per-package import restrictions

## Notes

Uses `swc` for fast TypeScript/JavaScript parsing and `turbo-trace` for import resolution. Produces actionable error messages with source locations via `miette`.
