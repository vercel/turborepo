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

## Test utilities

Downstream crates can enable the opt-in `test-util` feature in their
dev-dependencies and use
`turborepo_repository::test_util::{MockPackageDiscovery, MockPackageJsonLoader, PackageGraphFixture}`.
`PackageGraphBuilder::with_package_discovery` injects workspace discovery;
`with_package_json_loader` injects manifest loading. The production loader
continues to read `package.json` from disk by default. The fixture builder
injects both in-memory implementations and supports repository-relative package
directories, typed manifests, and internal dependencies. `build().await`
constructs a graph without walking workspaces or spawning `turbo`. External
resolution is skipped unless a lockfile is supplied with `with_lockfile`.

## Notes

Separated from `turborepo-cli` so the `@turbo/repository` NPM package can use it without pulling in the entire CLI. This crate is foundational - most other crates depend on it for package information.
