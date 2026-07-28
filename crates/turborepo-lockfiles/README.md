# turborepo-lockfiles

## Purpose

Package manager lockfile parsing, analysis, and serialization. Tracks which external packages workspace packages depend on for fine-grained cache invalidation.

## Architecture

```
Lockfile (npm, pnpm, yarn, bun)
    └── turborepo-lockfiles
        ├── Parse lockfile format
        ├── Build dependency subgraph per package
        └── Detect global vs package-specific changes
```

Supported formats:
- npm (`package-lock.json`)
- pnpm (`pnpm-lock.yaml`)
- Yarn 1 (`yarn.lock`)
- Yarn Berry (`yarn.lock` v2+)
- Bun (`bun.lockb`)

## Notes

Parsing is more robust than serialization. Serialization is primarily used by `turbo prune` and is more error-prone. The main value is detecting which packages are affected by lockfile changes, avoiding global cache invalidation.
