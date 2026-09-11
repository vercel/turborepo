# Boundaries

**Experimental feature** - See [RFC](https://github.com/vercel/turborepo/discussions/9435)

Full docs: https://turborepo.dev/docs/reference/boundaries

Boundaries enforce package isolation by detecting:

1. Imports of files outside the package's directory
2. Imports of packages not declared in `package.json` dependencies
3. Circular dependencies between packages in the workspace graph

## Usage

```bash
turbo boundaries
```

Run this to check for workspace violations across your monorepo.

## Circular package dependencies

Boundaries reports packages that cyclically depend on each other through their
`package.json` dependency declarations. The diagnostic includes the dependency
path and repeats the first package at the end to show where the cycle closes:

```text
Circular package dependency detected: @repo/pkg-a -> @repo/pkg-b -> @repo/pkg-c -> @repo/pkg-a
```

Remove one of the dependencies in the reported path to make the package graph
acyclic.

## Tags

Tags allow you to create rules for which packages can depend on each other.

### Adding Tags to a Package

```json
// packages/ui/turbo.json
{
  "tags": ["internal"]
}
```

### Configuring Tag Rules

Rules go in root `turbo.json`:

```json
// turbo.json
{
  "boundaries": {
    "tags": {
      "public": {
        "dependencies": {
          "deny": ["internal"]
        }
      }
    }
  }
}
```

This prevents `public`-tagged packages from importing `internal`-tagged packages.

### Rule Types

**Allow-list approach** (only allow specific tags):

```json
{
  "boundaries": {
    "tags": {
      "public": {
        "dependencies": {
          "allow": ["public"]
        }
      }
    }
  }
}
```

**Deny-list approach** (block specific tags):

```json
{
  "boundaries": {
    "tags": {
      "public": {
        "dependencies": {
          "deny": ["internal"]
        }
      }
    }
  }
}
```

**Restrict dependents** (who can import this package):

```json
{
  "boundaries": {
    "tags": {
      "private": {
        "dependents": {
          "deny": ["public"]
        }
      }
    }
  }
}
```

### Using Package Names

Package names work in place of tags:

```json
{
  "boundaries": {
    "tags": {
      "private": {
        "dependents": {
          "deny": ["@repo/my-pkg"]
        }
      }
    }
  }
}
```

## Key Points

- Rules apply transitively (dependencies of dependencies)
- Helps enforce architectural boundaries at scale
- Catches violations before runtime/build errors
