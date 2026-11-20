# turborepo-hash

## Purpose

Hashing utilities for Turborepo cache keys. Uses Cap'n Proto for deterministic cross-platform serialization, then applies xxHash64 for fast hashing.

Lockfile-package canonical serialization and hashing live in the cycle-free
`turborepo-lockfile-hash` crate. This crate retains the existing owned and
borrowed compatibility wrappers and delegates them to that primitive.

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
