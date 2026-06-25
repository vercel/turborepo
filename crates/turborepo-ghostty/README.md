# turborepo-ghostty

Ghostty-backed virtual terminal support for Turborepo's TUI.

This crate links against [libghostty-vt](https://ghostty.org) through
[`turborepo-ghostty-sys`](../turborepo-ghostty-sys), which owns the FFI
bindings and Zig build of Ghostty sources.

## Crates

- **`turborepo-ghostty-sys`** — raw FFI (`bindings.rs`) + `build.rs` that compiles libghostty-vt
- **`turborepo-ghostty`** — safe wrappers, `Parser`, and ratatui `TerminalWidget`

## Build requirements

Zig 0.15.2+ must be on `PATH` when building (CI installs it via `setup-zig`).

## Attribution

Most of the Rust code in this crate is vendored or adapted from upstream Ghostty
ecosystem projects. Turborepo-specific pieces (`Parser`, integration glue) were
written for this repo.

- **[Ghostty](https://github.com/ghostty-org/ghostty)** — terminal emulation
  via `libghostty-vt`, built by `turborepo-ghostty-sys`. License: MIT.
- **[libghostty-rs](https://github.com/Uzaaft/libghostty-rs)** — safe Rust
  wrappers in `alloc.rs`, `error.rs`, `fmt.rs`, `render.rs`, `screen.rs`,
  `selection.rs`, `style.rs`, and `terminal.rs` are adapted from the
  [`libghostty-vt`](https://github.com/Uzaaft/libghostty-rs/tree/master/crates/libghostty-vt)
  crate, trimmed to the API surface Turborepo needs. License: MIT OR
  Apache-2.0.
- **[ratatui-ghostty](https://codeberg.org/jint/ratatui-ghostty)** — `widget.rs`
  and `convert.rs` are adapted from its ratatui integration (terminal widget
  rendering and style conversion). License: MIT.
