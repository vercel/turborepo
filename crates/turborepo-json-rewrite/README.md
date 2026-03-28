# turborepo-json-rewrite

## Purpose

Minimal JSON/JSONC document mutation. Updates values at specified paths while preserving formatting, comments, and structure.

## Architecture

```
JSONC document + path + new value
    └── turborepo-json-rewrite
        ├── Parse to AST (preserving positions)
        ├── Find target path
        └── Minimal text replacement
            └── Modified document
```

## Notes

Designed to make surgical edits to JSON files without reformatting the entire document. Useful for programmatic updates to `turbo.json` and `package.json` while preserving user formatting and comments.
