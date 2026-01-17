# turborepo-config

## Purpose

Configuration loading and merging for Turborepo. Combines configuration from multiple sources with proper precedence.

## Architecture

```
Configuration sources (lowest to highest priority):
    1. turbo.json files
    2. Global config (~/.turbo/config.json)
    3. Local config (.turbo/config.json)
    4. Environment variables
    5. CLI arguments
        │
        └── Merged configuration
```

Key modules:
- `turbo_json/` - turbo.json file loading
- `file/` - Config file parsing
- `env/` - Environment variable handling
- `override_env/` - CLI argument overrides

## Notes

Later sources override earlier ones. This crate handles the complexity of finding, parsing, and merging all configuration sources into a single resolved configuration.
