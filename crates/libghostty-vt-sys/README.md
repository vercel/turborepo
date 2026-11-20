# libghostty-vt-sys (Turborepo patch)

Local patch of [`libghostty-vt-sys`](https://github.com/Uzaaft/libghostty-rs) with
Windows MSVC static-linking and portable CPU-target fixes. Rust bindings and support
track [PR84](https://github.com/Uzaaft/libghostty-rs/pull/84) at
`3c7766d4e5d6ce93870ad666a14c0b5a0e957360`, compatible with Ghostty
`5147c0a503cdeed7615cce245cdd815ed7a6b692`. The public header set differs from PR84's
Ghostty pin only in allocator documentation.

Upstream `0.2.1` emits `static=ghostty-vt`, which MSVC resolves to `ghostty-vt.lib` — the DLL
import library rather than the static archive. That leaves `turbo.exe` depending on
`ghostty-vt.dll` at runtime.

This patch links `ghostty-vt-static.lib` on Windows MSVC instead, matching
[vercel/turborepo#13171](https://github.com/vercel/turborepo/pull/13171).

Vendored builds also pass `-Dcpu=baseline`, as recommended by Ghostty for distributed
artifacts. Without it, Zig targets the build machine's native CPU and the resulting `turbo`
binary can crash with an illegal instruction on older CPUs.

Activated via a patch for the `libghostty-rs` git source in the workspace root
`Cargo.toml`, alongside the wrapper dependency pinned to the same PR84 revision.
Keep the local build portability patches when updating the Rust bindings.

The binding generator uses `LIBGHOSTTY_VT_SYS_INCLUDE_DIR`, emitted by `build.rs`,
so it reads the headers installed by that exact build rather than scanning potentially
stale output directories. `GHOSTTY_INCLUDE_DIR` or `GHOSTTY_SOURCE_DIR` can override
this path. Headers are parsed as C++11 so Ghostty's explicit signed `int` enum types
are honored even by libclang versions without fixed-enum support in C mode.
