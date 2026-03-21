# Turborepo with Cargo workspace providers

This example shows a **Rust-first** Turborepo that uses the Cargo workspace provider.

It is intentionally more realistic than a minimal fixture:

- A reusable Rust library crate (`math-core`)
- A CLI crate (`math-cli`) that depends on that library
- Real tests and buildable source files
- A provider-aware `turbo.json`

## Using this example

```sh
npx create-turbo@latest -e with-cargo
```

## What's inside?

```text
.
├── Cargo.toml
├── turbo.json
└── crates
    ├── math-core
    │   ├── src/lib.rs
    │   └── tests/statistics.rs
    └── math-cli
        └── src/main.rs
```

## Try it

### Build everything

```sh
turbo run build
```

### Run tests

```sh
turbo run test
```

### Filter to one crate

```sh
turbo run build --filter=math-cli
```

### See inferred commands without executing them

```sh
turbo run build --dry=json
```

You should see Cargo-inferred commands like `cargo build` in the dry-run output.
