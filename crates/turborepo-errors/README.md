# turborepo-errors

## Purpose

Diagnostic utilities for preserving source locations in error messages. Works with `miette` to provide actionable errors with source snippets.

## Architecture

```
turborepo-errors
    └── Spanned<T>
        ├── Value of type T
        ├── Source file path
        └── Span (start/end positions)
```

When errors occur, the span information enables rich error output:
```
Error: Invalid task name
  --> turbo.json:15:5
   |
15 |     "build#": { ... }
   |     ^^^^^^^^ task name cannot end with #
```

## Notes

Any parsing that might produce errors should use `Spanned<T>` to preserve location information. This enables significantly better error messages than just "invalid config".
