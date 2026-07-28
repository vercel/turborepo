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
        ├── package_json/ - package.json parsing
        └── discovery/ - Find all workspace packages
```

Key types:
- `PackageGraph` - Graph of workspace packages and their dependencies
- `PackageInfo` - Metadata about a single package
- `PackageManager` - Abstraction over npm/pnpm/yarn/bun

## Notes

Separated from `turborepo-lib` so the `@turbo/repository` NPM package can use it without pulling in the entire CLI. This crate is foundational - most other crates depend on it for package information.
