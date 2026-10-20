# Turborepo module federation starter

This is a community-maintained example. If you experience a problem, please submit a pull request with a fix. GitHub Issues will be closed.

## Using this example

Run the following command:

```sh
npx create-turbo@latest -e with-vite-module-federation
```

## What's inside?

This Turborepo includes the following packages/apps:

### Apps and Packages

- `react-host`: a [Vite](https://vite.dev/) React host app that consumes the remote app
- `react-remote`: a [Vite](https://vite.dev/) React remote app that exposes `./remote-app`
- `@mf-vite-example/shared-ui`: a stub React component library shared by both applications

Each package/app is 100% [TypeScript](https://www.typescriptlang.org/).

### Utilities

This Turborepo includes:

- [TypeScript](https://www.typescriptlang.org/) for static type checking
- [Vite](https://vite.dev/) for local development and production builds
- [Module Federation](https://module-federation.io/) for runtime composition between the host and remote applications

Install dependencies with the pinned package manager:

```sh
corepack enable
pnpm install
```

### Build

Build all apps and packages:

```sh
pnpm build
```

Build a specific app with a [filter](https://turborepo.dev/docs/crafting-your-repository/running-tasks#using-filters):

```sh
pnpm build:filter react-host
```

### Develop

Start both applications:

```sh
pnpm dev
```

Visit `http://localhost:4173`. The host app runs on port `4173` and the remote app runs on port `4174`.

Start a specific app with a filter:

```sh
pnpm dev:filter react-host
```

### Type checking

Type-check all workspaces:

```sh
pnpm typecheck
```

### Remote Caching

> [!TIP]
> Vercel Remote Cache is free for all plans. Get started at [vercel.com](https://vercel.com/signup?utm_source=remote-cache-sdk&utm_campaign=free_remote_cache).

To enable [Remote Caching](https://turborepo.dev/docs/core-concepts/remote-caching), authenticate and link the repository:

```sh
pnpm exec turbo login
pnpm exec turbo link
```

## Useful Links

- [Tasks](https://turborepo.dev/docs/crafting-your-repository/running-tasks)
- [Caching](https://turborepo.dev/docs/crafting-your-repository/caching)
- [Remote Caching](https://turborepo.dev/docs/core-concepts/remote-caching)
- [Filtering](https://turborepo.dev/docs/crafting-your-repository/running-tasks#using-filters)
- [Configuration Options](https://turborepo.dev/docs/reference/configuration)
- [CLI Usage](https://turborepo.dev/docs/reference/command-line-reference)
