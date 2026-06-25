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

Bindings were generated from libghostty-vt 0.2.0 (MIT OR Apache-2.0).
