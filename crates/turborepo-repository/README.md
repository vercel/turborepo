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

## Notes

Separated from `turborepo-cli` so the `@turbo/repository` NPM package can use it without pulling in the entire CLI. This crate is foundational - most other crates depend on it for package information.
