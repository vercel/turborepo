# turborepo-diagnostics

## Purpose

Diagnostic infrastructure for Turborepo health checks and troubleshooting. Provides a framework for running diagnostics and reporting results.

## Architecture

```
turborepo-diagnostics
    ├── Diagnostic trait - Interface for checks
    ├── DiagnosticChannel - Communication
    └── Built-in diagnostics
        ├── Git FS Monitor check
        ├── Daemon health check
        ├── LSP status check
        └── Update availability check
```

Diagnostics can:
- Report status messages
- Request user input
- Suspend terminal output for interactive prompts

## Notes

Extracted from `turborepo-lib` to reduce coupling. Used by `turbo daemon status` and other troubleshooting commands.
