# turborepo

## Purpose

The main Turborepo binary crate. This is a thin wrapper that sets up panic handling and delegates to `turborepo-lib` for all actual functionality.

## Architecture

```
turborepo (binary)
    └── turborepo-lib (all CLI logic)
```

The binary itself contains minimal code - just the `main()` entry point. All CLI parsing, command execution, and core logic lives in `turborepo-lib`.

## Notes

This separation exists for historical reasons from the Go-to-Rust migration. During migration, keeping the binary thin allowed building Rust code without triggering Go builds. The split could be collapsed but hasn't been prioritized.
