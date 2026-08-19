# Turborepo Filter Syntax Reference

## Running Only Changed Packages: `--affected`

**The primary way to run only changed packages is `--affected`:**

```bash
# Run build/test/lint only in changed packages and their dependents
turbo run build test lint --affected
```

This compares your current branch to `main` (falling back to `master`) and runs tasks in:

1. Packages with file changes
2. Packages that depend on changed packages (dependents)

### Why Include Dependents?

If you change `@repo/ui`, packages that import `@repo/ui` (like `apps/web`) need to re-run their tasks to verify they still work with the changes.

### Customizing --affected

```bash
# Use a different base branch
TURBO_SCM_BASE=origin/develop turbo run build --affected

# Use a different head (current state)
TURBO_SCM_HEAD=HEAD~5 turbo run build --affected
```

Base resolution order: `TURBO_SCM_BASE`, then the CI base ref on GitHub Actions (`GITHUB_BASE_REF` for PRs, the push event's previous SHA otherwise — errors if unresolvable rather than falling through), then the literal refs `main`, `master`. Turborepo does NOT read the repo's configured default branch — if the default branch is anything else (e.g. `develop`), set `TURBO_SCM_BASE`.

### Common CI Pattern

```yaml
# .github/workflows/ci.yml
- run: turbo run build test lint --affected
```

This is the most efficient CI setup - only run tasks for what actually changed.

---

## Manual Git Comparison with --filter

For more control, use `--filter` with git comparison syntax:

```bash
# Changed packages + dependents (same as --affected)
turbo run build --filter=...[origin/main]

# Only changed packages (no dependents)
turbo run build --filter=[origin/main]

# Changed packages + dependencies (packages they import)
turbo run build --filter=[origin/main]...

# Changed since last commit
turbo run build --filter=...[HEAD^1]

# Changed between two commits
turbo run build --filter=[a1b2c3d...e4f5g6h]
```

### Comparison Syntax

| Syntax        | Meaning                               |
| ------------- | ------------------------------------- |
| `[ref]`       | Packages changed since `ref`          |
| `...[ref]`    | Changed packages + their dependents   |
| `[ref]...`    | Changed packages + their dependencies |
| `...[ref]...` | Dependencies, changed, AND dependents |

---

## Other Filter Types

Filters select which packages to include in a `turbo run` invocation.

### Basic Syntax

```bash
turbo run build --filter=<package-name>
turbo run build -F <package-name>
```

Multiple filters combine as a union (packages matching ANY filter run).

### By Package Name

```bash
--filter=web          # exact match
--filter=@acme/*      # scope glob
--filter=*-app        # name glob
```

### By Directory

```bash
--filter=./apps/*           # all packages in apps/
--filter=./packages/ui      # specific directory
```

### By Dependencies/Dependents

| Syntax      | Meaning                                |
| ----------- | -------------------------------------- |
| `pkg...`    | Package AND all its dependencies       |
| `...pkg`    | Package AND all its dependents         |
| `...pkg...` | Dependencies, package, AND dependents  |
| `^pkg...`   | Only dependencies (exclude pkg itself) |
| `...^pkg`   | Only dependents (exclude pkg itself)   |

### Negation

Exclude packages with `!`:

```bash
--filter=!web              # exclude web
--filter=./apps/* --filter=!admin   # apps except admin
```

### Task Identifiers

Run a specific task in a specific package:

```bash
turbo run web#build        # only web's build task
turbo run web#build api#test   # web build + api test
```

### Combining Filters

Multiple `--filter` flags create a union:

```bash
turbo run build --filter=web --filter=api   # runs in both
```

---

## Quick Reference: Changed Packages

| Goal                               | Command                                                    |
| ---------------------------------- | ---------------------------------------------------------- |
| Changed + dependents (recommended) | `turbo run build --affected`                               |
| Custom base branch                 | `TURBO_SCM_BASE=origin/develop turbo run build --affected` |
| Only changed (no dependents)       | `turbo run build --filter=[origin/main]`                   |
| Changed + dependencies             | `turbo run build --filter=[origin/main]...`                |
| Since last commit                  | `turbo run build --filter=...[HEAD^1]`                     |
