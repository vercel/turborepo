# Contributing

## Build Instructions

- build the `./crates/turborepo-lsp` binary
- move it into `./packages/turbo-vsc/out`, with a name such as `turborepo-lsp-darwin-arm64`
- bundle the app using `pnpm package`

## Marketplace Release

The Turborepo release workflow calls the reusable `LSP` GitHub Actions workflow after creating a release tag to package and publish the VSIX artifact. The `LSP` workflow can also be run manually with `publish=false` and `dry_run=true` to package without publishing.

Publishing requires `publish=true`, `dry_run=false`, and a `VSCE_PAT` secret on the protected `vscode-marketplace` environment.

VS Code extension versions must use `major.minor.patch`, so the workflow maps Turborepo versions before publishing:

- Stable `M.m.p` publishes as `M.m.(p * 1000)`.
- Canary `M.m.p-canary.n` publishes as `M.m.((p - 1) * 1000 + n + 1)` with `--pre-release`.
- Canary versions for a future patch-zero release decrement the preceding segment, so `M.m.0-canary.n` publishes as `M.(m - 1).(999000 + n + 1)` with `--pre-release`.

For example, `2.9.10-canary.0` publishes as `2.9.9001`, `2.9.10` publishes as `2.9.10000`, and `2.10.0-canary.0` publishes as `2.9.999001`.
