# turborepo-fs

## Purpose

File system utilities for Turborepo. Currently focused on recursive directory copying with gitignore support, primarily used by `turbo prune`.

## Architecture

```
turborepo-fs
    └── recursive_copy()
        ├── Walks directory tree
        ├── Optionally respects .gitignore
        └── Preserves file metadata
```

Uses `ignore` crate for efficient gitignore-aware walking.

## Notes

A focused utility crate. Most filesystem operations use `turbopath` directly; this crate exists for more complex operations like recursive copying that need gitignore awareness.
