# turborepo-hash

## Purpose

Hashing utilities for Turborepo cache keys. Uses Cap'n Proto for deterministic cross-platform serialization, then applies xxHash64 for fast hashing.

## Architecture

```
Input data (env vars, file contents, task config)
    └── Cap'n Proto serialization (deterministic)
        └── xxHash64
            └── Cache key (hash string)
```

Key types:
- `TurboHash` trait - Implemented by types that contribute to cache keys
- `TaskHashable` - Task-specific inputs for hashing
- `GlobalHashable` - Repository-wide inputs

## Notes

Cap'n Proto ensures identical inputs produce identical hashes across platforms and Rust/Go implementations (historical). xxHash64 provides fast, high-quality hashing for the serialized data.
