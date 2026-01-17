# turborepo-cache

## Purpose

Cache management for task outputs. Provides both local filesystem caching and remote caching via the Vercel API, with a multiplexer that coordinates between both.

## Architecture

```
turborepo-cache
    ├── AsyncCache (worker pool wrapper)
    │   └── Multiplexer
    │       ├── fs/ (local cache)
    │       │   └── .turbo/cache/<hash>.tar.zst
    │       └── http/ (remote cache)
    │           └── Vercel Remote Cache API
    ├── cache_archive/ (tar.zst packing/unpacking)
    └── signature_authentication/ (optional signing)
```

Cache artifacts are gzipped tarballs. When both local and remote caches are enabled:
- Reads prefer local cache
- Writes go to both (remote writes are async)

## Notes

Supports optional signature authentication for cache artifacts via private keys. The `AsyncCache` wrapper enables non-blocking cache operations during task execution.
