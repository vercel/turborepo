# turborepo-scope

Package scope resolution for Turborepo. Filters and selects packages based on `--filter` patterns, `--affected` change detection, and glob matching.

## Architecture

```
                           resolve_packages()
                                  |
                                  v
                          +---------------+
                          | FilterResolver|
                          +---------------+
                                  |
          +-----------+----------+-----------+
          |           |                      |
          v           v                      v
   +-----------+ +-----------+      +------------------+
   | TargetSel-| | SimpleGlob|      | ScopeChange-     |
   | ector     | | (names)   |      | Detector         |
   +-----------+ +-----------+      +------------------+
        |                                    |
        v                                    v
   Parse filter strings             Query SCM for changed
   like "foo...",                   files between refs,
   "{dir}[ref]"                     map to packages
```

**Modules:**
- `filter.rs` - Core `FilterResolver` that combines selectors, globs, and change detection
- `target_selector.rs` - Parses `--filter` syntax (`...pkg...`, `{dir}`, `[ref]`)
- `change_detector.rs` - `GitChangeDetector` trait + `ScopeChangeDetector` implementation
- `simple_glob.rs` - Lightweight glob for package name matching (not paths)

## Notes

- Extracted from `turborepo-lib` to reduce coupling
- The `ResolutionError` type is large (~128 bytes) due to `ChangeMapError`; boxing was intentionally avoided for CLI ergonomics
- `ScopeOpts` is re-exported from `turborepo-types` for backwards compatibility
- `SimpleGlob` is intentionally minimal - use `wax::Glob` for path matching
