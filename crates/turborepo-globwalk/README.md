# turborepo-globwalk

## Purpose

Glob pattern matching and directory walking for Turborepo. Wraps `wax` with Turborepo-specific corrections and character escaping for user-provided globs.

## Architecture

```
User glob string
    └── turborepo-globwalk
        ├── Escape special characters wax doesn't support
        ├── Normalize glob syntax
        └── wax (actual matching)
```

Provides:
- `ValidatedGlob` - A glob that's been validated and normalized
- Directory walking with include/exclude patterns
- Support for files, folders, or both

## Notes

Handles edge cases in user-provided globs that would otherwise fail in `wax`. The escaping logic is important for globs containing characters that `wax` treats as special but Turborepo users expect to be literal.
