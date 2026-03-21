# Turborepo with uv workspace providers

This example shows a **Python-first** Turborepo that uses the uv workspace provider.

It includes real package structure and tests:

- `mathlib`: reusable utility package
- `analytics`: package that depends on `mathlib`
- provider-aware task execution via `turbo run`

## Using this example

```sh
npx create-turbo@latest -e with-uv
```

## What's inside?

```text
.
├── pyproject.toml
├── turbo.json
└── packages
    ├── mathlib
    │   ├── pyproject.toml
    │   ├── src/mathlib/__init__.py
    │   └── tests/test_mathlib.py
    └── analytics
        ├── pyproject.toml
        ├── src/analytics/report.py
        └── tests/test_report.py
```

## Try it

### Build workspace packages

```sh
turbo run build
```

### Run tests

```sh
turbo run test
```

### Run lint or formatting

```sh
turbo run lint
turbo run fmt
```

### Inspect inferred commands

```sh
turbo run build --dry=json
```

You should see uv-inferred commands like `uv build` in the dry-run output.
