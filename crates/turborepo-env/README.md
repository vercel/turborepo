# turborepo-env

## Purpose

Environment variable filtering and hashing for tasks. Filters environment variables based on task configuration and produces deterministic hashes for cache keys.

## Architecture

```
Process environment
    └── turborepo-env
        ├── Filter by patterns (include/exclude)
        ├── Apply strict/loose mode
        └── Hash for cache key
```

Filtering modes:
- **Strict**: Only explicitly listed env vars
- **Loose**: All process env vars available

## Notes

Environment variables are a key cache input. This crate ensures deterministic hashing (sorted, consistent format) and proper filtering based on `turbo.json` configuration.
