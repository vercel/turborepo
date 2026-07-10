# AGENTS.md

Instructions for AI agents working on this repository.

## Architecture

See [ARCHITECTURE.md](./crates/turborepo/ARCHITECTURE.md) for an overview of the `turbo run` command architecture.

## Keeping Documentation Up to Date

When making changes to the codebase, check if the following docs need updates:

- **[ARCHITECTURE.md](./crates/turborepo/ARCHITECTURE.md)** - Update when changing core `turbo run` components:
  - Run builder, package graph, task graph/engine
  - Task visitor, caching system, task hashing
  - Run tracking and summary generation
  - Any files in `crates/turborepo-lib/src/run/`, `crates/turborepo-lib/src/engine/`, `crates/turborepo-lib/src/task_graph/`, or `crates/turborepo-cache/`

- **[CONTRIBUTING.md](./CONTRIBUTING.md)** - Update when changing:
  - Build process or development setup
  - Testing procedures or requirements
  - Project structure or tooling

- **This file (AGENTS.md)** - Update when changing:
  - PR requirements or CI workflows
  - Repository conventions or policies

## Pull Request Guidelines

### Always run pre-commit/pre-push hooks

- You are not allowed to use `--no-verify` when making a commit or push.
- If you do not have dependencies available, you can download them with `pnpm install --frozen-lockfile`.

### Rust panic extraction policy

- Workspace Clippy lints deny `.unwrap()`, `.unwrap_err()`, `.unwrap_none()`, and `.expect()` in Rust targets covered by `cargo lint`.
- Crates with existing implementation-code violations may temporarily allow `clippy::unwrap_used` and `clippy::expect_used` at the crate root; remove those allows as each crate is cleaned up.
- Tests are exempt from this panic-extraction policy, but still linted by `cargo lint` with panic-extraction lints allowed under `cfg(test)`.

### CI task scheduling

- Test and lint workflows do not pre-classify changed paths. PR jobs run consistently and use the Turborepo task graph and cache where applicable.
- Same-repository PRs authenticate to Remote Cache through OIDC; fork PRs remain local-only.
- Rust CI is dogfooding full Cargo target restoration on Ubuntu while repository sccache dogfooding is disabled. The draft branch temporarily retains its PR cache across commits for measurement.
- Example validation remains push-only because it requires Vercel credentials and project state.

### PR Title Format

PR titles must follow [Conventional Commits](https://www.conventionalcommits.org/). See [`.github/workflows/lint-pr-title.yml`](./.github/workflows/lint-pr-title.yml) for the enforced constraints.

Format: `<type>: <Description>`

Key rules:

- Description must start with an uppercase letter
- Scopes are not allowed

Examples:

```
feat: Add new cache configuration option
fix: Resolve race condition in task scheduling
docs: Update installation instructions
```

## Release Workflow Notes

- The `LSP` workflow packages `packages/turbo-vsc` VSIX artifacts for release. Stable and canary Turborepo versions are mapped to Marketplace-safe `major.minor.patch` versions before packaging.
- Canary VS Code extension packages use `--pre-release`.
- Non-dry-run releases publish the VS Code extension through the `LSP` workflow using `publish=true`, `dry_run=false`, and a `VSCE_PAT` secret on the protected `vscode-marketplace` environment. This publish path must not block release PR creation or cleanup published npm release state.
- The `Release` workflow signs and notarizes macOS `turbo` binaries during `build-rust` using static GitHub secrets and `apple-codesign`/`rcodesign`.
- The `Release` and `LSP` workflows install Zig during `build-rust` because `turbo` and `turborepo-lsp` link `libghostty-vt` through `libghostty-vt-sys`.
