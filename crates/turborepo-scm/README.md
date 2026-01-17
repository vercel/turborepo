# turborepo-scm

## Purpose

Source control integration for Turborepo. Provides git operations for finding changed files, retrieving previous lockfile versions, and hashing files efficiently.

## Architecture

```
turborepo-scm
    ├── git/ - Git operations via CLI and libgit2
    │   ├── Changed file detection
    │   ├── File hashing (ls-tree, hash-object)
    │   └── Previous file versions
    ├── package_deps/ - Package-level change detection
    └── worktree/ - Git worktree info
```

Two backends:
- Git CLI commands (more compatible)
- `git2` bindings via feature flag (faster for some operations)

## Notes

SCM integration enables `--affected` filtering and efficient file hashing. When the daemon is unavailable, SCM-based hashing is the fallback. Requires git 2.18+ for full functionality.
