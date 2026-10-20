# turborepo-schema-gen

## Purpose

Generates JSON Schema and TypeScript type definitions from Rust types. Ensures the published schema and types stay in sync with the Rust source of truth.

## Architecture

```
Rust types (turborepo-turbo-json)
    └── turborepo-schema-gen
        ├── schema subcommand → schema.json
        └── typescript subcommand → types.ts
```

Uses `schemars` for JSON Schema generation and `ts-rs` for TypeScript.

## Notes

This is a CLI binary, not a library. Run it to regenerate schema files when turbo.json types change. The `verify` subcommand can check if generated files are up-to-date.
