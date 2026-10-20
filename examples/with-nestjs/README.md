# Turborepo starter

This is a community-maintained example. If you experience a problem, please submit a pull request with a fix. GitHub Issues will be closed.

## Using this example

Run the following command:

```bash
npx create-turbo@latest -e with-nestjs
```

## What's inside?

This Turborepo includes the following packages and apps:

### Apps and Packages

```shell
.
├── apps
│   ├── api                       # NestJS app (https://nestjs.com).
│   └── web                       # Next.js app (https://nextjs.org).
└── packages
    ├── @repo/api                 # Shared NestJS resources.
    ├── @repo/eslint-config       # ESLint configurations (includes Prettier)
    ├── @repo/jest-config         # Jest configurations
    ├── @repo/typescript-config   # tsconfig.json files used throughout the monorepo
    └── @repo/ui                  # Shareable React component library.
```

Each package and application is mostly written in [TypeScript](https://www.typescriptlang.org/).

### Utilities

This Turborepo has some additional tools already set up for you:

- [TypeScript](https://www.typescriptlang.org/) for static type safety
- [ESLint](https://eslint.org/) for code linting
- [Prettier](https://prettier.io) for code formatting
- [Jest](https://jestjs.io/) for testing

### Commands

This Turborepo includes useful commands for its apps and packages.

#### Build

```bash
# Build all apps and packages that have a `build` script.
pnpm build
```

#### Develop

```bash
# Run development servers for all apps and packages that have a `dev` script.
pnpm dev
```

#### Test

```bash
# Run unit tests for all apps and packages that have a `test` script.
pnpm test

# Run end-to-end tests for all apps and packages that have a `test:e2e` script.
pnpm test:e2e
```

See `@repo/jest-config` to customize test behavior.

#### Lint

```bash
# Lint all apps and packages that have a `lint` script.
pnpm lint
```

See `@repo/eslint-config` to customize lint behavior.

#### Check types

```bash
# Type-check all apps and packages that have a `check-types` script.
pnpm check-types
```

#### Format

```bash
# Format supported TypeScript, TSX, and Markdown files.
pnpm format
```

See `@repo/eslint-config/prettier-base` to customize formatting.

### Remote Caching

> [!TIP]
> Vercel Remote Cache is free for all plans. Get started today at [vercel.com](https://vercel.com/signup?/signup?utm_source=remote-cache-sdk&utm_campaign=free_remote_cache).

Turborepo can use [Remote Caching](https://turborepo.dev/docs/core-concepts/remote-caching) to share cache artifacts across machines and CI.

By default, Turborepo caches locally. To enable Remote Caching, create a [Vercel account](https://vercel.com/signup?utm_source=turborepo-examples), then authenticate:

```bash
pnpm turbo login
```

Next, link the repository to Remote Cache from the repository root:

```bash
pnpm turbo link
```

## Useful Links

This example takes inspiration from the [with-nextjs](https://github.com/vercel/turborepo/tree/main/examples/with-nextjs) Turborepo example and the [01-cats-app](https://github.com/nestjs/nest/tree/master/sample/01-cats-app) NestJS sample.

Learn more about Turborepo:

- [Tasks](https://turborepo.dev/docs/crafting-your-repository/running-tasks)
- [Caching](https://turborepo.dev/docs/crafting-your-repository/caching)
- [Remote Caching](https://turborepo.dev/docs/core-concepts/remote-caching)
- [Filtering](https://turborepo.dev/docs/crafting-your-repository/running-tasks#using-filters)
- [Configuration Options](https://turborepo.dev/docs/reference/configuration)
- [CLI Usage](https://turborepo.dev/docs/reference/command-line-reference)
