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

Two options for authenticating to [Vercel Remote Cache](https://vercel.com/docs/monorepos/remote-caching):

- **OIDC (recommended)**: GitHub OIDC token exchanged for a short-lived Turborepo access token. No long-lived secrets in your workflow.
- **Personal Access Token (PAT)**: Long-lived token stored as a GitHub secret.

### Option A: OpenID Connect (recommended)

#### 1. Create a Turborepo CLI OIDC Policy

On vercel.com, go to your team's **Settings → Build and Deployment → OIDC Policies for CLI Access**. Click **Add** next to **Turborepo CLI Policies** and fill out the form (policy name, GitHub account, repository). You can optionally restrict to a specific workflow or branch and customize the audience.

#### 2. Add `TURBO_TEAM` Repository Variable

Settings > Secrets and variables > Actions > Variables:

- `TURBO_TEAM`: Your Vercel team ID (`team_123…`), from your team's General settings page.

Use a repository variable rather than a secret so GitHub Actions doesn't censor your team ID in logs.

#### 3. Add the Action to Your Workflow

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

      - uses: vercel/setup-turborepo-remote-cache-action@v1
        with:
          team-id: ${{ vars.TURBO_TEAM }}
```

The action requests a GitHub OIDC token, exchanges it for a short-lived Turborepo access token, and sets `TURBO_TOKEN` and `TURBO_TEAM` for subsequent steps. Must run before any step that invokes `turbo`.

### Option B: Personal Access Token

#### 1. Create Vercel Access Token

1. Go to [Vercel Dashboard](https://vercel.com/account/tokens)
2. Create a new token with appropriate scope
3. Copy the token value

#### 2. Add Secrets and Variables

In your GitHub repository settings:

**Secrets** (Settings > Secrets and variables > Actions > Secrets):

- `TURBO_TOKEN`: Your Vercel access token

**Variables** (Settings > Secrets and variables > Actions > Variables):

- `TURBO_TEAM`: Your Vercel team slug

#### 3. Add to Workflow

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

      - uses: vercel/setup-turborepo-remote-cache-action@v1
        with:
          team-id: ${{ vars.TURBO_TEAM }}

      - name: Install dependencies
        run: pnpm install --frozen-lockfile

      - name: Build
        run: turbo run build --affected

      - name: Test
        run: turbo run test --affected

      - name: Lint
        run: turbo run lint --affected
```
