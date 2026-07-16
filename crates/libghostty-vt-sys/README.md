# libghostty-vt-sys (Turborepo patch)

Vendored copy of [`libghostty-vt-sys`](https://crates.io/crates/libghostty-vt-sys) 0.2.0 with
Windows MSVC static-linking and portable CPU-target fixes.

Upstream `0.2.0` emits `static=ghostty-vt`, which MSVC resolves to `ghostty-vt.lib` — the DLL
import library rather than the static archive. That leaves `turbo.exe` depending on
`ghostty-vt.dll` at runtime.

This patch links `ghostty-vt-static.lib` on Windows MSVC instead, matching
[vercel/turborepo#13171](https://github.com/vercel/turborepo/pull/13171).

Vendored builds also pass `-Dcpu=baseline`, as recommended by Ghostty for distributed
artifacts. Without it, Zig targets the build machine's native CPU and the resulting `turbo`
binary can crash with an illegal instruction on older CPUs.

Activated via `[patch.crates-io]` in the workspace root `Cargo.toml`. Remove this crate once
upstream `libghostty-vt-sys` includes both fixes.
