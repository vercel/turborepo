# Turborepo starter with Yarn Berry

This is a community-maintained example. If you experience a problem, please submit a pull request with a fix. GitHub Issues will be closed.

## Using this example

Use Node.js 26.11.1 or newer and Yarn 4.18.1 (pinned in `packageManager`). Install [Corepack](https://yarnpkg.com/corepack) if it is not available, then enable its Yarn shim:

```sh
npm install --global corepack
corepack enable
npx create-turbo@latest -e with-berry
cd my-turborepo
yarn install
```

Corepack selects the project's pinned Yarn version; no vendored Yarn binary is needed. This example uses Yarn's `node-modules` linker.

## What's inside?

This Turborepo uses [modern Yarn (Berry)](https://yarnpkg.com/) as its package manager. It includes:

- `docs`: a [Next.js](https://nextjs.org/) App Router app on port 3001
- `web`: another Next.js App Router app on port 3000
- `@repo/ui`: a React component library shared as source by both apps
- `@repo/eslint-config`: shared ESLint flat configurations with TypeScript, Next.js, Turborepo, and Prettier-compatible rules
- `@repo/typescript-config`: shared TypeScript configurations

Workspace dependencies use Yarn's `workspace:*` protocol.

### Utilities

- [TypeScript](https://www.typescriptlang.org/) for static type checking
- [ESLint](https://eslint.org/) for code linting
- [Prettier](https://prettier.io) for code formatting

### Build

From the repository root:

```sh
yarn build
```

Builds run after type checking, including generation of Next.js route types.

### Develop

```sh
yarn dev
```

Edit `apps/web/app/page.tsx` or `apps/docs/app/page.tsx` to get started.

### Lint and type check

```sh
yarn lint
yarn check-types
```

### Remote Caching

> [!TIP]
> Vercel Remote Cache is free for all plans. Get started today at [vercel.com](https://vercel.com/signup?utm_source=remote-cache-sdk&utm_campaign=free_remote_cache).

Turborepo's [Remote Caching](https://turborepo.dev/docs/core-concepts/remote-caching) shares cache artifacts across machines and CI/CD pipelines.

Turborepo caches locally by default. To enable Remote Caching, create a [Vercel account](https://vercel.com/signup?utm_source=turborepo-examples), then run from the repository root:

```sh
yarn turbo login
yarn turbo link
```

## Useful Links

- [Tasks](https://turborepo.dev/docs/crafting-your-repository/running-tasks)
- [Caching](https://turborepo.dev/docs/crafting-your-repository/caching)
- [Remote Caching](https://turborepo.dev/docs/core-concepts/remote-caching)
- [Filtering](https://turborepo.dev/docs/crafting-your-repository/running-tasks#using-filters)
- [Configuration Options](https://turborepo.dev/docs/reference/configuration)
- [CLI Usage](https://turborepo.dev/docs/reference/run)
