# turborepo-turbo-json

## Purpose

Parsing and validation for `turbo.json` configuration files. The source of truth for task definitions, pipeline configuration, and repository settings.

## Architecture

```
turbo.json file
    └── turborepo-turbo-json
        ├── parser/ - JSONC parsing (comments allowed)
        ├── raw/ - Direct deserialization types
        ├── processed/ - Validated, resolved types
        └── validator/ - Configuration validation
```

Key types:
- `RawTurboJson` - Unprocessed turbo.json structure
- `ProcessedTaskDefinition` - Validated task configuration
- `TurboJsonLoader` - Handles loading and extending configs

Supports:
- JSONC (JSON with comments)
- `extends` for configuration inheritance
- Root and package-level turbo.json files

## Notes

Validation is strict - invalid configurations produce actionable error messages with source locations via `miette`. The raw/processed split allows for gradual validation and better error reporting.
