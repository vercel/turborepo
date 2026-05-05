# Contributing

## Build Instructions

- build the `./crates/turborepo-lsp` binary
- move it into `./packages/turbo-vsc/out`, with a name such as `turborepo-lsp-darwin-arm64`
- bundle the app using `pnpm package`

## Marketplace Release

The Turborepo release workflow calls the reusable `LSP` GitHub Actions workflow after creating a release tag and release PR. The `LSP` workflow can also be run manually with `publish=true` and `dry_run=false`.

Publishing requires a `VSCE_PAT` secret on the protected `vscode-marketplace` environment. Dry runs package and upload the VSIX without publishing.

VS Code extension versions must use `major.minor.patch`, so the workflow maps Turborepo versions before publishing:

- Stable `M.m.p` publishes as `M.m.(p * 1000)`.
- Canary `M.m.p-canary.n` publishes as `M.m.((p - 1) * 1000 + n + 1)` with `--pre-release`.

For example, `2.9.10-canary.0` publishes as `2.9.9001`, and `2.9.10` publishes as `2.9.10000`.
