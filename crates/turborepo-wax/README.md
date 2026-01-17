# turborepo-wax

## Purpose

Vendored fork of the [wax](https://github.com/olson-sean-k/wax) glob library. Provides opinionated, portable glob pattern matching with consistent semantics across platforms.

## Architecture

```
turborepo-wax
    ├── Glob parsing and compilation
    ├── Pattern matching against paths
    └── Directory tree walking with glob filters
```

Used by `turborepo-globwalk` to provide the underlying glob semantics. The glob syntax emphasizes component boundaries - `*` never crosses path separators, only `**` does.

Key features:
- Forward slash `/` is the only separator (portable)
- Supports `*`, `**`, `?`, character classes `[...]`, alternatives `{a,b}`, and repetitions `<pattern:n,m>`
- Case sensitivity via flags: `(?i)` for case-insensitive matching

## Notes

This is a vendored copy to allow Turborepo-specific modifications. Changes should be kept minimal to ease upstream syncing.
