# turborepo-ghostty-sys

Raw FFI bindings for [libghostty-vt](https://ghostty.org), the virtual terminal
library extracted from [Ghostty](https://ghostty.org).

At build time this crate fetches a pinned Ghostty commit and compiles
`libghostty-vt` via Zig. Checked-in `src/bindings.rs` provides the Rust FFI
definitions.

## Build requirements

- Zig 0.15.2 or newer on `PATH`
- `git` for vendored source fetch

Optional environment variables:

- `GHOSTTY_SOURCE_DIR` — use a local Ghostty checkout instead of fetching
- `GHOSTTY_ZIG_SYSTEM_DIR` — pre-populated Zig package cache for offline builds
- `TURBOREPO_GHOSTTY_SYS_OPTIMIZE` — override Zig optimize mode

## Attribution

This crate vendors FFI and build logic in-tree rather than depending on the
upstream crates.io packages. The following projects were used as inspiration
and source material:

- **[Ghostty](https://github.com/ghostty-org/ghostty)** — `libghostty-vt` is
  compiled from Ghostty sources at build time. License: MIT.
- **[libghostty-rs](https://github.com/Uzaaft/libghostty-rs)** — `src/bindings.rs`
  and `build.rs` are adapted from the
  [`libghostty-vt-sys`](https://github.com/Uzaaft/libghostty-rs/tree/master/crates/libghostty-vt-sys)
  crate (bindings generated from libghostty-vt 0.2.0). License: MIT OR
  Apache-2.0.
