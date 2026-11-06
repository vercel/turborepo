# Turborepo starter with Yarn Classic

This is a community-maintained example. If you experience a problem, please submit a pull request with a fix. GitHub Issues will be closed.

## Using this example

Use Node.js 26.13.0 or newer and Yarn Classic 1.22.22 (the version pinned in `packageManager`).

```sh
npx create-turbo@latest my-turborepo -e with-yarn --package-manager yarn
cd my-turborepo
yarn install
```

If Yarn is not installed, install the pinned version with `npm install --global yarn@1.22.22`. This example intentionally uses Yarn Classic rather than Yarn Berry.

## What's inside?

This Turborepo includes the following packages/apps:

### Apps and Packages

- `docs`: a [Next.js](https://nextjs.org/) app on port 3001
- `web`: another [Next.js](https://nextjs.org/) app on port 3000
- `@repo/ui`: a stub React component library shared by both `web` and `docs` applications
- `@repo/eslint-config`: shared flat ESLint configurations for Next.js and React packages
- `@repo/typescript-config`: `tsconfig.json`s used throughout the monorepo

Each package/app is 100% [TypeScript](https://www.typescriptlang.org/).

### Utilities

This Turborepo has some additional tools already set up for you:

- [TypeScript](https://www.typescriptlang.org/) for static type checking
- [ESLint](https://eslint.org/) for code linting
- [Prettier](https://prettier.io) for code formatting

### Build

From the repository root, build all apps and packages:

```sh
yarn build
```

### Develop

From the repository root, start both development servers:

```sh
yarn dev
```

### Lint and type-check

Lint source files and check TypeScript separately from the repository root:

```sh
yarn lint
yarn check-types
```

The apps generate Next.js route types before type-checking, so no prior build is required.

### Remote Caching

> [!TIP]
> Vercel Remote Cache is free for all plans. Get started today at [vercel.com](https://vercel.com/signup?/signup?utm_source=remote-cache-sdk&utm_campaign=free_remote_cache).

Turborepo can use a technique known as [Remote Caching](https://turborepo.dev/docs/core-concepts/remote-caching) to share cache artifacts across machines, enabling you to share build caches with your team and CI/CD pipelines.

By default, Turborepo will cache locally. To enable Remote Caching you will need an account with Vercel. If you don't have an account you can [create one](https://vercel.com/signup?utm_source=turborepo-examples), then run the following from the repository root:

```sh
yarn turbo login
```

This will authenticate the Turborepo CLI with your [Vercel account](https://vercel.com/docs/concepts/personal-accounts/overview).

Next, link your Turborepo to your Remote Cache:

```sh
yarn turbo link
```

## Useful Links

Learn more about the power of Turborepo:

- [Tasks](https://turborepo.dev/docs/crafting-your-repository/running-tasks)
- [Caching](https://turborepo.dev/docs/crafting-your-repository/caching)
- [Remote Caching](https://turborepo.dev/docs/core-concepts/remote-caching)
- [Filtering](https://turborepo.dev/docs/crafting-your-repository/running-tasks#using-filters)
- [Configuration Options](https://turborepo.dev/docs/reference/configuration)
- [CLI Usage](https://turborepo.dev/docs/reference/command-line-reference)
