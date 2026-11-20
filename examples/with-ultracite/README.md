# Turborepo with Ultracite

This example is maintained by the community.

## Using this example

Use Node.js 26.13.0 or newer and pnpm 12.16.1, as pinned in
`devEngines.packageManager` in `package.json`.

Run the following commands:

```sh
npx create-turbo@latest my-turborepo -e with-ultracite --package-manager pnpm
cd my-turborepo
pnpm install
```

## What's inside?

This Turborepo includes the following packages/apps:

### Apps and Packages

- `docs`: a [Next.js](https://nextjs.org/) app on port 3001
- `web`: another [Next.js](https://nextjs.org/) app on port 3000
- `@repo/ui`: a stub React component library shared by both `web` and `docs` applications
- `@repo/typescript-config`: `tsconfig.json`s used throughout the monorepo

Each package/app is 100% [TypeScript](https://www.typescriptlang.org/).

### Utilities

This Turborepo has some additional tools already set up for you:

- [TypeScript](https://www.typescriptlang.org/) for static type checking
- [Ultracite](https://ultracite.ai/) for code linting and formatting (powered by [Biome](https://biomejs.dev/))

Run the following commands from the monorepo root.

### Build

To build all apps and packages:

```sh
pnpm build
```

### Develop

To develop all apps and packages:

```sh
pnpm dev
```

### Linting and Formatting

To check all apps and packages for linting and formatting issues:

```sh
pnpm check
```

To automatically fix linting and formatting issues:

```sh
pnpm fix
```

### Type Checking

To generate Next.js route types and check all apps and packages:

```sh
pnpm check-types
```

To run every validation task together through Turborepo, including the root-level Ultracite check:

```sh
pnpm turbo run build check-types check
```

### Remote Caching

> [!TIP]
> Vercel Remote Cache is free for all plans. Get started today at [vercel.com](https://vercel.com/signup?utm_source=remote-cache-sdk&utm_campaign=free_remote_cache).

Turborepo can use a technique known as [Remote Caching](https://turborepo.dev/docs/core-concepts/remote-caching) to share cache artifacts across machines, enabling you to share build caches with your team and CI/CD pipelines.

By default, Turborepo will cache locally. To enable Remote Caching you will need an account with Vercel. If you don't have an account you can [create one](https://vercel.com/signup?utm_source=turborepo-examples), then enter the following command from the monorepo root:

```sh
pnpm turbo login
```

This will authenticate the Turborepo CLI with your [Vercel account](https://vercel.com/docs/concepts/personal-accounts/overview).

Next, you can link your Turborepo to your Remote Cache by running:

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
