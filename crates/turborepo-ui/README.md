# turborepo-ui

## Purpose

Terminal UI library for Turborepo. Handles colors, spinners, progress indicators, log output, and the interactive TUI mode.

## Architecture

```
turborepo-ui
    ├── Colors and styling (via console crate)
    ├── PrefixedUI - Output with task prefixes
    ├── ColorSelector - Assign colors to concurrent tasks
    ├── LogWriter - Task log handling
    ├── tui/ - Interactive terminal UI (ratatui)
    └── wui/ - Web UI server
```

Key components:
- `ColorConfig` - Terminal color configuration
- `OutputClient` - Manages where output goes
- `TaskTable` - TUI task status display
- Log replay for cached task output

## Notes

Supports multiple output modes: streaming, grouped, and errors-only. The TUI mode provides an interactive view of task execution. Includes panic recovery to restore terminal state if something goes wrong.
