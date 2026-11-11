# turbo-trace

## Purpose

Import dependency tracing for JavaScript and TypeScript files. Discovers and tracks module imports across a codebase. It is the import tracer behind `turbo boundaries` and `turbo query`'s file queries. The `turbo trace` command is separate: it lives in the `turborepo-trace` crates.

## Architecture

```
Source file
    └── turbo-trace
        ├── ImportFinder - AST-based import extraction
        └── Tracer - Follows import chains
            └── Import graph
```

Identifies import types:
- ES modules (`import`)
- CommonJS (`require`)
- Dynamic imports

## Notes

Used by the boundaries feature to enforce architectural constraints on which packages can import from which other packages.
