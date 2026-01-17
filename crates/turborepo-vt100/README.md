# turborepo-vt100

## Purpose

Terminal emulator parser library. Parses terminal byte streams and provides an in-memory representation of rendered contents. Used by the TUI to capture and display task output.

## Architecture

```
Terminal byte stream
    └── Parser
        └── Screen (in-memory representation)
            ├── Cell grid with colors/attributes
            ├── Cursor position
            └── Diff calculation for efficient updates
```

Essentially the parser component of a terminal emulator, useful for:
- Capturing terminal output from child processes
- Rendering terminal content in the TUI
- Computing minimal diffs for screen updates

## Notes

This is a vendored fork of the `vt100` crate with Turborepo-specific modifications. Based on commit `1e4014aa72a7552d2f69b81ad89d56e035354041`. Fuzz tests were dropped during vendoring. Changes should ideally be upstreamed.
