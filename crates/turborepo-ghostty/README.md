# turborepo-ghostty

Ghostty-backed virtual terminal support for Turborepo's TUI.

This crate depends on [`libghostty-vt`](https://crates.io/crates/libghostty-vt) for
safe Rust bindings to [libghostty-vt](https://ghostty.org), and adds
Turborepo-specific integration:

- **`Parser`** — high-level API for task output parsing, scrolling, and selection
- **`TerminalWidget`** — ratatui widget for rendering terminal state

## Build requirements

Zig 0.15.2+ must be on `PATH` when building (CI installs it via `setup-zig`).
`libghostty-vt-sys` fetches and compiles Ghostty sources at build time.

On Windows MSVC, Turborepo patches `libghostty-vt-sys` (via `[patch.crates-io]`) so release
binaries link `ghostty-vt-static.lib` instead of the DLL import library. See
`crates/libghostty-vt-sys/README.md`.

## Attribution

- **[Ghostty](https://github.com/ghostty-org/ghostty)** — terminal emulation via
  `libghostty-vt`. License: MIT.
- **[libghostty-rs](https://github.com/Uzaaft/libghostty-rs)** — Rust bindings
  (`libghostty-vt`, `libghostty-vt-sys`). License: MIT OR Apache-2.0.
- **[ratatui-ghostty](https://codeberg.org/jint/ratatui-ghostty)** — `widget.rs`
  and `convert.rs` are adapted from its ratatui integration. License: MIT.
