# turborepo-fixed-map

## Purpose

Specialized map with fixed keys determined at construction. Provides thread-safe, one-time initialization of values for predefined keys.

## Architecture

```
FixedMap<K, V>
    ├── Keys fixed at construction
    ├── Values initialized lazily (OnceLock)
    └── Thread-safe concurrent access
```

Operations:
- `new(keys)` - Create with known keys
- `get(key)` - Get value if initialized
- `insert(key, value)` - Initialize value (first write wins)

## Notes

Used for lazy-loading `turbo.json` files. The fixed key set allows caching loaded configs while the `OnceLock` ensures thread-safe initialization. Values cannot be removed or overwritten once set.
