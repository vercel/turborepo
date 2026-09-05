# GitHub Actions

Complete setup guide for Turborepo with GitHub Actions.

## Basic Workflow Structure

```yaml
name: CI

on:
  push:
    branches: [main]
  pull_request:
    branches: [main]

jobs:
  build:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
        with:
          fetch-depth: 2

      - uses: actions/setup-node@v4
        with:
          node-version: 20

      - name: Install dependencies
        run: npm ci

      - name: Build and Test
        run: turbo run build test lint
```

## Package Manager Setup

### pnpm

```yaml
- uses: pnpm/action-setup@v3
  with:
    version: 9

- uses: actions/setup-node@v4
  with:
    node-version: 20
    cache: "pnpm"

- run: pnpm install --frozen-lockfile
```

### Yarn

```yaml
- uses: actions/setup-node@v4
  with:
    node-version: 20
    cache: "yarn"

- run: yarn install --frozen-lockfile
```

### Bun

```yaml
- uses: oven-sh/setup-bun@v1
  with:
    bun-version: latest

- run: bun install --frozen-lockfile
```

## Remote Cache Setup

There are two ways to authenticate to [Vercel Remote Cache](https://vercel.com/docs/monorepos/remote-caching) from GitHub Actions:

- **OpenID Connect (OIDC)**: configure a policy that allows exchanging the CI/CD provider's OIDC tokens for short-lived Turborepo access tokens that grant access to Remote Cache (recommended).
- **Personal Access Token (PAT)**: configure a long-lived, team-scoped PAT as a secret in your CI/CD provider (use when OIDC is not an option).

For the full setup, see [Use Remote Caching from external CI/CD](https://vercel.com/docs/monorepos/remote-caching/external-ci-cd). The GitHub Actions workflow configuration is shown below.

### Option A: OpenID Connect (recommended)

Follow [these instructions](https://vercel.com/docs/monorepos/remote-caching/external-ci-cd#openid-connect-oidc) to create an OIDC policy on your Vercel team and configure your `TURBO_TEAM` variable. Then add [`vercel/setup-turborepo-remote-cache-action`](https://github.com/vercel/setup-turborepo-remote-cache-action) before any step that runs `turbo`. It requests a GitHub OIDC token, exchanges it for a short-lived Turborepo access token, and sets `TURBO_TOKEN` and `TURBO_TEAM` environment variables for later steps that run `turbo`:

```yaml
jobs:
  build:
    runs-on: ubuntu-latest
    permissions:
      contents: read
      id-token: write

    steps:
      - uses: actions/checkout@v4
        with:
          fetch-depth: 2

      - uses: vercel/setup-turborepo-remote-cache-action@v1.1.0
        with:
          team: ${{ vars.TURBO_TEAM }}
```

### Option B: Personal Access Token

Follow [these instructions](https://vercel.com/docs/monorepos/remote-caching/external-ci-cd#personal-access-token-pat) to create a Personal Access Token, configure your `TURBO_TOKEN` secret, and configure your `TURBO_TEAM` variable. Then provide them to jobs that run `turbo`:

```yaml
jobs:
  build:
    runs-on: ubuntu-latest
    env:
      TURBO_TOKEN: ${{ secrets.TURBO_TOKEN }}
      TURBO_TEAM: ${{ vars.TURBO_TEAM }}
```

## Alternative: actions/cache

If you can't use remote cache, cache Turborepo's local cache directory:

```yaml
- uses: actions/cache@v4
  with:
    path: .turbo
    key: turbo-${{ runner.os }}-${{ hashFiles('**/turbo.json', '**/package-lock.json') }}
    restore-keys: |
      turbo-${{ runner.os }}-
```

Note: This is less effective than remote cache since it's per-branch.

## Complete Example

```yaml
name: CI

on:
  push:
    branches: [main]
  pull_request:
    branches: [main]

jobs:
  build:
    runs-on: ubuntu-latest
    permissions:
      contents: read
      id-token: write

    steps:
      - uses: actions/checkout@v4
        with:
          fetch-depth: 2

      - uses: pnpm/action-setup@v3
        with:
          version: 9

      - uses: actions/setup-node@v4
        with:
          node-version: 20
          cache: "pnpm"

      - uses: vercel/setup-turborepo-remote-cache-action@v1.1.0
        with:
          team: ${{ vars.TURBO_TEAM }}

      - name: Install dependencies
        run: pnpm install --frozen-lockfile

      - name: Build
        run: turbo run build --affected

      - name: Test
        run: turbo run test --affected

      - name: Lint
        run: turbo run lint --affected
```
