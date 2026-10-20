# turborepo-microfrontends

## Purpose

Configuration parsing for `@vercel/microfrontends`. Extracts the minimal information Turborepo needs to run a local development proxy.

## Architecture

```
microfrontends.json
    └── turborepo-microfrontends
        ├── TurborepoMfeConfig (minimal extraction)
        │   ├── Default package
        │   ├── Package names
        │   └── Dev task names
        └── Convert to full Config for proxy
```

## Notes

Intentionally parses only what Turborepo needs, avoiding tight coupling with `@vercel/microfrontends` internals. Vercel-specific fields are passed through but ignored by Turborepo.
