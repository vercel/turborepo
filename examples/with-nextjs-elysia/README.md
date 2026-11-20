# Turborepo starter: Next.js + Elysia

This is a community-maintained example. If you experience a problem, please submit a pull request with a fix. GitHub Issues will be closed.

## Using this example

Run the following command:

```bash
npx create-turbo@latest -e with-nextjs-elysia
```

## What's inside?

This Turborepo includes the following packages & apps:

### Apps and Packages

```shell
.
├── apps
│   └── web                       # Next.js app (https://nextjs.org) with an embedded Elysia server
└── packages
    ├── @repo/contract            # Shared TypeBox contracts and Drizzle ORM schemas
    ├── @repo/typescript-config   # `tsconfig.json`s used throughout the monorepo
    └── @repo/ui                  # Shareable stub React component library.
```

Each package and application are mostly written in [TypeScript](https://www.typescriptlang.org/).

### Tech Stack

- **Frontend**: Next.js 16 with the App Router
- **Backend**: [Elysia](https://elysiajs.com), mounted in a Next.js catch-all route at `/api/v1` and called with [Eden Treaty](https://elysiajs.com/eden/overview)
- **Database**: PostgreSQL with Drizzle ORM
- **API Contract**: TypeBox for runtime type validation
- **Package Manager**: [Bun](https://bun.sh)

### Utilities

This `Turborepo` has some additional tools already set for you:

- [TypeScript](https://www.typescriptlang.org/) for static type-safety
- [Biome](https://biomejs.dev/) with [Ultracite](https://www.ultracite.ai/) presets for code linting and formatting
- [Drizzle ORM](https://orm.drizzle.team/) for database operations

### Commands

This `Turborepo` already configured useful commands for all your apps and packages.

#### Build

```bash
# Will build all the app & packages with the supported `build` script.
bun run build
```

#### Develop

```bash
# Will run the development server for all the app & packages with the supported `dev` script.
bun run dev
```

#### Lint

```bash
# Will lint all the app & packages with the supported `lint` script.
bun run lint
```

#### Type check

```bash
# Will type check all the app & packages with the supported `check-types` script.
bun run check-types
```

#### Format

```bash
# Will apply Biome lint fixes and formatting across the whole repository.
bun run fix
```

#### Database

```bash
# Set your database URL in `packages/contract/drizzle.config.ts`, then push the schema.
bun run --filter @repo/contract db:push
```

### Remote Caching

> [!TIP]
> Vercel Remote Cache is free for all plans. Get started today at [vercel.com](https://vercel.com/signup?utm_source=remote-cache-sdk&utm_campaign=free_remote_cache).

Turborepo can use a technique known as [Remote Caching](https://turborepo.dev/docs/core-concepts/remote-caching) to share cache artifacts across machines, enabling you to share build caches with your team and CI/CD pipelines.

By default, Turborepo will cache locally. To enable Remote Caching you will need an account with Vercel. If you don't have an account you can [create one](https://vercel.com/signup?utm_source=turborepo-examples), then enter the following commands:

```bash
bunx turbo login
```

This will authenticate the Turborepo CLI with your [Vercel account](https://vercel.com/docs/concepts/personal-accounts/overview).

Next, you can link your Turborepo to your Remote Cache by running the following command from the root of your Turborepo:

```bash
bunx turbo link
```

## Useful Links

Learn more about the power of Turborepo:

- [Tasks](https://turborepo.dev/docs/crafting-your-repository/running-tasks)
- [Caching](https://turborepo.dev/docs/crafting-your-repository/caching)
- [Remote Caching](https://turborepo.dev/docs/core-concepts/remote-caching)
- [Filtering](https://turborepo.dev/docs/crafting-your-repository/running-tasks#using-filters)
- [Configuration Options](https://turborepo.dev/docs/reference/configuration)
- [CLI Usage](https://turborepo.dev/docs/reference/command-line-reference)
