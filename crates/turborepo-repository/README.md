# turborepo-repository

## Purpose

Repository detection, package discovery, and package graph construction. Understands monorepo structure, workspace configurations, and inter-package dependencies.

## Architecture

```
Repository root
    └── turborepo-repository
        ├── inference/ - Detect repo type and package manager
        ├── package_manager/ - npm, pnpm, yarn, bun support
        ├── package_graph/ - Dependency graph of workspace packages
        ├── external_resolution.rs - Explicit external dependency resolution domains
        ├── package_json/ - package.json parsing
        └── discovery/ - Find all workspace packages
```

Key types:
- `PackageGraph` - Graph of workspace packages and their dependencies
- `RepositoryKnowledge` - Immutable authority for package and aggregate identities, paths, kinds, and toolchain provenance
- `PackageManager` - Abstraction over npm/pnpm/yarn/bun
- `ExternalResolutionDomain` - Immutable domain identity, membership, and resolution data

## Dependency injection

`PackageGraphBuilder::with_package_discovery` accepts any `PackageDiscovery`
implementation to supply workspace paths and the package manager.
`with_package_json_loader` accepts a `PackageJsonLoader` to resolve each
discovered manifest. Both traits accept closures, so a downstream test can
inject in-memory inputs without implementing boilerplate mock structs or
spawning `turbo`. By default, local discovery and filesystem manifest loading
retain their existing behavior. Other toolchains can be injected through
`with_contributor`.

## Repository bootstrap contract

A repository root is not a JavaScript package. A recognized, enabled native
workspace does not require a root `package.json` or a JavaScript package manager.
Adding incidental JavaScript metadata must not erase its native scopes.

`RepositoryBootstrap::observe` produces a `RootObservation`: optional JavaScript
facts plus a `ContributorPlan` of recognized native roots and their registered
factories. Inference uses this observation for eligibility and membership; graph
construction consumes the same plan without probing roots again. JavaScript
manifest and workspace policy belongs to `bootstrap::javascript`, not the
ancestor-selection loop. Missing, disabled, recognized, and invalid definitions
are distinct: malformed participating manifests retain their original errors.

Plans are construction-scoped observations, not persistent filesystem caches.
Rebuild at process/config boundaries and after graph-defining watch changes.
Custom configuration paths select configuration content and flags, not a new
repository root. Inference must honor declared membership and Git boundaries.
Setup applies stricter write-safety boundaries, but declared native members do
not become independent roots merely by containing `package.json`.

### Adding an ecosystem

1. Implement `ToolchainBootstrap` and register its flag and factory together in
   the bootstrap registry. Do not add ecosystem branches to inference, optional
   root loading, or graph construction.
2. Root probing and membership inventories must be subprocess-free and must not
   require the ecosystem's executable. Reuse the contributor's authoritative
   workspace parser and inventory rules, including exclusions.
3. Full contributor discovery owns tasks, dependencies, and tool execution.
   Identity-only consumers must not load those facts merely to list packages.
4. Run the shared bootstrap/inference tests and extend the CLI contract matrix in
   `crates/turborepo/tests/repository_bootstrap_test.rs`. Cover roots without
   JavaScript manifests, incidental root metadata, members with JavaScript
   manifests, custom configs, disabled adapters, and invalid definitions.
5. Preserve the fake fourth-ecosystem tests: another adapter must work through
   inference and eager/lazy graph construction without adding core switches.

Single-package mode is a JavaScript execution optimization, not permission to
skip other contributors. Resolve it consistently from the requested mode and
observed execution scopes; configuration loading, engine construction, summaries,
and watch definitions must agree. A root-only empty JavaScript workspace must
not be silently reclassified as a standalone package.

Focused contract checks:

```sh
cargo test --offline --locked -p turborepo-repository --lib bootstrap
cargo test --offline --locked -p turborepo-repository --lib inference
cargo test --offline --locked -p turbo --test repository_bootstrap_test
```

## Notes

Separated from `turborepo-cli` so the `@turbo/repository` NPM package can use it without pulling in the entire CLI. This crate is foundational - most other crates depend on it for package information.
