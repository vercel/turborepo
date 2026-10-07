# Vendored terminal-control 0.6.0 test harness

This is a dev-only library subset of the published `terminal-control` 0.6.0
crate, retained for Turborepo's existing black-box TUI tests. It is not a
production terminal wrapper, an upstream upgrade, or a replacement CLI.

## Exact origin

- Registry package: <https://crates.io/crates/terminal-control/0.6.0>
- Upstream repository: <https://github.com/anomalyco/terminal-control>
- Published `.cargo_vcs_info.json` revision:
  `e9e3744fd62d81f2f600fbd9681d254a3b54d37d` (repository root).
- Copied on 2026-10-07 from the unpacked published crate archive.
- Cached published `terminal-control-0.6.0.crate` archive SHA-256:
  `a47a63b6a923cf498ff503d74b4430908f1f1b659a1b459958822a5c67c457e0`.

`LICENSE`, `THIRD_PARTY_LICENSES.md`, and `README.md` are copied verbatim. The
MIT copyright and permission notices for Terminal Control, Ghostty, and
libghostty-rs are preserved. The upstream README describes the full published
package; its CLI/MCP/install instructions and toolchain requirements are not
instructions for this local subset.

## Retained footprint

The existing TUI test imports `frame::{Cell, Color, Frame}`, `session::Session`,
and `shot::{Options, Shot}`. Keep those modules and their complete source-module
closure: `recording`, `render` (including `render/box_drawing.rs`), `runtime`,
`semantic`, `terminal_core`, and `terminal_theme`, plus `workspace`. These are
retained whole, including their upstream tests, rather than rewriting the
session, PTY lifecycle, input handling, frame capture, logging, or rendering.

Omitted as unnecessary for this harness: `src/main.rs`, `src/driver.rs`,
`src/mcp.rs`, JSON schemas, the published lockfile/original manifest, and Cargo
registry bookkeeping files. The retained library does not embed the schemas.

## Local modifications

1. `Cargo.toml` is derived from the published normalized manifest. Keep version
   `0.6.0`, edition 2024, Rust 1.93 minimum, licensing metadata, and dependency
   versions/features for the retained modules. Disable publishing, remove the
   `termctrl` binary and its CLI/MCP-only dependencies (`clap`, `rmcp`, `tokio`),
   and update the include list for the retained files and this provenance note.
2. Pin `libghostty-vt` to Git repository
   `https://github.com/uzaaft/libghostty-rs`, exact revision
   `3c7766d4e5d6ce93870ad666a14c0b5a0e957360`, retaining
   `default-features = false`. The registry 0.2.1 wrapper uses the obsolete
   `TerminalOptions` API and is incompatible with the parent's new sys bindings.
3. `src/lib.rs` omits only the `driver` and `mcp` module declarations.
4. `src/terminal_core.rs` drops the `TerminalOptions` import and calls
   `Terminal::new(cols, rows)`. Before processing any output, zero scrollback
   sets `set_scrollback_max_bytes(Some(0))` to disable history; a zero line cap
   alone still retains historical pages. Nonzero scrollback sets
   `set_scrollback_max_lines(Some(max_scrollback))`, preserving the original
   line-count meaning (including the 10,000-row session limit) and Ghostty's
   default byte cap. Both setters propagate errors with context. All other
   terminal callbacks, formatting, resize, frame/style, and theme APIs remain
   unchanged after static comparison with the pinned Rust API.
5. Add two `terminal_core` regression tests: zero scrollback excludes history
   before/after resize, and nonzero scrollback preserves the configured line
   limit, default byte cap, retained text, and visible frame.
6. `.rustfmt.toml` selects default formatting locally, preventing the parent's
   import, comment, and string-formatting rules from rewriting upstream source.
7. `src/session.rs` permits dead code only for the retained
   `show_target_with_status` helper, whose upstream caller belongs to the omitted
   CLI, so workspace linting can keep warnings denied.
8. `src/workspace.rs` gives the pending-action queue an initial capacity of eight
   entries to follow the parent's allocation lint policy without changing queue
   semantics.

All other retained Rust sources are byte-for-byte copies of the publication.
No production wrapper or existing TUI test is modified.

## Integration and verification

This dev-only subset is a workspace member so Turborepo can hash and prune its
local path dependency safely. It is consumed through `crates/turborepo`'s
platform-gated dev dependency. Its Ghostty wrapper and sys
bindings must resolve to the same revisions as the production integration.
Keep the root Git-source sys override when updating dependencies.

Run these checks from the repository root so Cargo applies the workspace's
native build configuration and dependency patches:

```sh
cargo test -p terminal-control --lib --locked
cargo test -p turbo --test tui_test --locked
```

The harness tests include its retained upstream coverage and the two added
scrollback regression tests. The black-box suite exercises the built `turbo`
binary through real terminal sessions.
