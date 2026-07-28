# turbo-trace

## Purpose

Import dependency tracing for JavaScript and TypeScript files. Discovers and tracks module imports across a codebase. Powers `turbo boundaries` functionality.

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
