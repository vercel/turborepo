# turborepo-dirs

## Purpose

Platform-specific directory resolution for configuration files. Wraps `dirs_next` with Turborepo-specific overrides via environment variables.

## Architecture

```
config_dir()
    ├── Check TURBO_CONFIG_DIR_PATH env var
    │   └── If set: use that path
    └── Fall back to dirs_next::config_dir()

vercel_config_dir()
    ├── Check VERCEL_CONFIG_DIR_PATH env var
    │   └── If set: use that path
    └── Fall back to dirs_next::config_dir()
```

Returns `AbsoluteSystemPathBuf` for type safety.

## Notes

Environment variable overrides are useful for testing and non-standard installations. Empty strings are rejected per Unix convention.
