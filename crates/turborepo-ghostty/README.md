# turborepo-ghostty

Ghostty-backed virtual terminal support for Turborepo's TUI.

This crate links against [libghostty-vt](https://ghostty.org) through
[`turborepo-ghostty-sys`](../turborepo-ghostty-sys), which owns the FFI
bindings and Zig build of Ghostty sources. The safe Rust wrappers in `src/`
are adapted from [libghostty-rs](https://github.com/uzaaft/libghostty-rs)
(MIT OR Apache-2.0), trimmed to the API surface Turborepo needs.

## Crates

- **`turborepo-ghostty-sys`** — raw FFI (`bindings.rs`) + `build.rs` that compiles libghostty-vt
- **`turborepo-ghostty`** — safe wrappers, `Parser`, and ratatui `TerminalWidget`

## Build requirements

Zig 0.15.2+ must be on `PATH` when building (CI installs it via `setup-zig`).
