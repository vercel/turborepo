# Turborepo starter with Rollup

This is a community-maintained example. If you experience a problem, please submit a pull request with a fix. GitHub Issues will be closed.

## Using this example

Run the following command:

```sh
npx create-turbo@latest -e with-rollup
```

## What's inside?

This Turborepo includes the following packages/apps:

### Apps and Packages

- `web`: a [Next.js](https://nextjs.org) app
- `@repo/eslint-config`: `eslint` flat configurations (includes `@next/eslint-plugin-next` and `eslint-config-prettier`)
- `@repo/typescript-config`: `tsconfig.json`s used throughout the monorepo
- `@repo/ui`: a React component library used by the `web` application, compiled with Rollup

Each package/app is 100% [TypeScript](https://www.typescriptlang.org/).

### Utilities

This Turborepo has some additional tools already set up for you:

- [TypeScript](https://www.typescriptlang.org/) for static type checking
- [ESLint](https://eslint.org/) for code linting
- [Prettier](https://prettier.io) for code formatting

### Build

To build all apps and packages, run the following command:

```sh
cd my-turborepo
pnpm build
```

### Develop

To develop all apps and packages, run the following command:

```sh
cd my-turborepo
pnpm dev
```

### Remote Caching

> [!TIP]
> Vercel Remote Cache is free for all plans. Get started today at [vercel.com](https://vercel.com/signup?utm_source=remote-cache-sdk&utm_campaign=free_remote_cache).

Turborepo can use [Remote Caching](https://turborepo.com/docs/core-concepts/remote-caching) to share cache artifacts across machines, enabling you to share build caches with your team and CI/CD pipelines.

By default, Turborepo caches locally. To enable Remote Caching, you need a Vercel account. If you don't have an account, you can [create one](https://vercel.com/signup?utm_source=turborepo-examples), then enter the following commands:

```sh
cd my-turborepo
pnpm turbo login
```

This authenticates the Turborepo CLI with your [Vercel account](https://vercel.com/docs/accounts).

Next, link your repository to Remote Cache by running the following command from the root of your Turborepo:

```sh
pnpm turbo link
```

## Useful Links

Learn more about the power of Turborepo:

- [Documentation](https://turborepo.com/docs)
- [Running tasks](https://turborepo.com/docs/crafting-your-repository/running-tasks)
- [Caching](https://turborepo.com/docs/crafting-your-repository/caching)
- [Remote Caching](https://turborepo.com/docs/core-concepts/remote-caching)
