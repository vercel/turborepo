# Turborepo starter

This Turborepo starter is maintained by the Turborepo core team.

## Using this example

Create a new Turborepo from this example:

```sh
pnpm dlx create-turbo@latest --example with-biome
```

## What's inside?

This Turborepo includes the following packages/apps:

### Apps and Packages

- `docs`: a [Next.js](https://nextjs.org/) app
- `web`: another [Next.js](https://nextjs.org/) app
- `@repo/ui`: a stub React component library shared by both `web` and `docs` applications
- `@repo/biome-config`: shared [Biome](https://biomejs.dev/) configurations
- `@repo/typescript-config`: shared `tsconfig.json` files

Each package/app is 100% [TypeScript](https://www.typescriptlang.org/).

### Utilities

This Turborepo has some additional tools set up for you:

- [TypeScript](https://www.typescriptlang.org/) for static type checking
- [Biome](https://biomejs.dev/) for code linting
- [Prettier](https://prettier.io) for code formatting

### Build

To build all apps and packages, run:

```sh
pnpm build
```

You can build a specific package by using a [filter](https://turborepo.dev/docs/crafting-your-repository/running-tasks#using-filters):

```sh
pnpm turbo build --filter=docs
```

### Develop

To develop all apps and packages, run:

```sh
pnpm dev
```

You can develop a specific package by using a filter:

```sh
pnpm turbo dev --filter=web
```

### Remote Caching

> [!TIP]
> Vercel Remote Cache is free for all plans. Get started today at [vercel.com](https://vercel.com/signup?utm_source=remote-cache-sdk&utm_campaign=free_remote_cache).

Turborepo can use [Remote Caching](https://turborepo.dev/docs/core-concepts/remote-caching) to share cache artifacts across machines, enabling you to share build caches with your team and CI/CD pipelines.

By default, Turborepo caches locally. To enable Remote Caching, create a [Vercel account](https://vercel.com/signup?utm_source=turborepo-examples), then authenticate the Turborepo CLI:

```sh
pnpm turbo login
```

Next, link the repository to your Remote Cache:

```sh
pnpm turbo link
```

## Useful Links

Learn more about the power of Turborepo:

- [Tasks](https://turborepo.dev/docs/crafting-your-repository/running-tasks)
- [Caching](https://turborepo.dev/docs/crafting-your-repository/caching)
- [Remote Caching](https://turborepo.dev/docs/core-concepts/remote-caching)
- [Filtering](https://turborepo.dev/docs/crafting-your-repository/running-tasks#using-filters)
- [Configuration Options](https://turborepo.dev/docs/reference/configuration)
- [CLI Usage](https://turborepo.dev/docs/reference/command-line-reference)
