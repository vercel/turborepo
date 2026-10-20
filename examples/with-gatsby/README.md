# Turborepo Gatsby.js starter

This is a community-maintained example. If you experience a problem, please submit a pull request with a fix. GitHub Issues will be closed.

## Using this example

Use Node.js 26.11.1 or newer and pnpm 12.11.2. Install the declared package manager and create the workspace:

```sh
npm install --global pnpm@12.11.2
npx create-turbo@2.11.8 -e with-gatsby
cd my-turborepo
pnpm install
```

The workspace explicitly approves the install scripts needed by Gatsby and its native dependencies in `pnpm-workspace.yaml`.

## What's inside?

### Apps and Packages

- `docs`: a [Next.js](https://nextjs.org/) Pages Router app
- `web`: a [Gatsby.js](https://www.gatsbyjs.com/docs) app
- `@repo/ui`: a React component library shared as TypeScript source by both applications
- `@repo/eslint-config`: shared ESLint flat configurations, including the Next.js plugin, Turbo rules, and Prettier compatibility
- `@repo/typescript-config`: shared TypeScript configurations

Both apps use React 19. Gatsby compiles `@repo/ui` with `gatsby-plugin-compile-es6-packages`; Next.js uses `transpilePackages`.

### Utilities

- [TypeScript](https://www.typescriptlang.org/) for static type checking
- [ESLint](https://eslint.org/) for code linting
- [Prettier](https://prettier.io) for code formatting

### Build

From the workspace root:

```sh
pnpm build
```

### Develop

```sh
pnpm dev
```

Open the Gatsby app at [http://localhost:8000](http://localhost:8000) and the Next.js app at [http://localhost:3001](http://localhost:3001).

### Lint and type check

```sh
pnpm lint
pnpm check-types
```

To run all non-persistent checks together:

```sh
pnpm exec turbo run build lint check-types
```

### Preview production builds

After building, run `pnpm --filter web start` for Gatsby at port 9000 or `pnpm --filter docs start` for Next.js at port 3001.

### Remote Caching

> [!TIP]
> Vercel Remote Cache is free for all plans. Get started today at [vercel.com](https://vercel.com/signup?utm_source=remote-cache-sdk&utm_campaign=free_remote_cache).

Turborepo can use [Remote Caching](https://turborepo.dev/docs/core-concepts/remote-caching) to share cache artifacts across machines, enabling your team and CI/CD pipelines to share build caches.

By default, Turborepo caches locally. To enable Remote Caching, create a [Vercel account](https://vercel.com/signup?utm_source=turborepo-examples), then run these commands from the workspace root using the installed CLI:

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
