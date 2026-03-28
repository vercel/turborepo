# turborepo-gitignore

## Purpose

Ensures the `.turbo` directory is listed in `.gitignore`. Automatically adds the entry if missing when Turborepo runs.

## Architecture

```
ensure_turbo_is_gitignored()
    ├── Check if .gitignore exists
    │   └── If not: create with .turbo entry
    └── If exists: check for .turbo entry
        └── If missing: append .turbo entry
```

## Notes

Simple utility that runs on startup to prevent accidentally committing cache artifacts.
