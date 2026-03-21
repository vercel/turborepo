# Turborepo with mixed workspace providers (Node + Cargo + uv)

This example demonstrates a single Turborepo containing:

- a Node app (`apps/web`)
- Rust crates (`crates/*`)
- Python packages (`packages/*`)

and runs them all through one task graph using:

```json
"workspaceProviders": ["node", "cargo", "uv"]
```

## Using this example

```sh
npx create-turbo@latest -e with-workspace-providers
```

## What this example demonstrates

- **Node scripts** are used for Node workspaces (`package.json` scripts)
- **Cargo tasks** are inferred for Rust workspaces (`Cargo.toml`)
- **uv tasks** are inferred for Python workspaces (`pyproject.toml`)
- You can still use one command surface:
  - `turbo run build`
  - `turbo run test`
  - `turbo run lint`

## Try it

### 1) Inspect task commands without executing

```sh
turbo run build --dry=json
```

Look for:

- `web#build` → Node script command
- `rust-app#build` → `cargo build`
- `py-app#build` → `uv build`

### 2) Run by provider-specific filter

```sh
turbo run build --filter=web
turbo run build --filter=rust-app
turbo run build --filter=py-app
```

### 3) Run entire mixed graph

```sh
turbo run build test lint
```
