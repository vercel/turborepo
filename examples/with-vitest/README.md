# Turborepo starter

This Turborepo starter is maintained by the Turborepo core team.

## Using this example

This example is based on the `basic` example (`npx create-turbo@latest`) to demonstrate how to use Vitest and get the most out of Turborepo's caching.

This example demonstrates two approaches to Vitest configuration:

1. **Package-level caching (Recommended)**: Each package has its own Vitest configuration that imports shared settings from `@repo/vitest-config`. This approach leverages Turborepo's caching effectively.

2. **Vitest Projects**: A root `vitest.config.ts` uses Vitest's projects feature for unified test running during development.

## Getting Started

First, install dependencies and build the shared configuration package:

```bash
pnpm install
pnpm build --filter=@repo/vitest-config
```

## Available Commands

- `pnpm test`: Runs tests in each package using Turborepo (leverages caching)
- `pnpm test:projects`: Runs tests using Vitest's projects feature
- `pnpm test:projects:watch`: Runs tests using Vitest's projects feature in watch mode
- `pnpm view-report`: Collects coverage from each package and shows it in a merged report

## Configuration Structure

The example uses a shared `@repo/vitest-config` package that exports:

- `sharedConfig`: Base configuration with coverage settings
- `baseConfig`: For Node.js packages (like `math`)
- `uiConfig`: For packages requiring jsdom environment (like `web`, `docs`)

### Remote Caching

> [!TIP]
> Vercel Remote Cache is free for all plans. Get started today at [vercel.com](https://vercel.com/signup?/signup?utm_source=remote-cache-sdk&utm_campaign=free_remote_cache).

Turborepo can use a technique known as [Remote Caching](https://turborepo.com/docs/core-concepts/remote-caching) to share cache artifacts across machines, enabling you to share build caches with your team and CI/CD pipelines.

By default, Turborepo will cache locally. To enable Remote Caching you will need an account with Vercel. If you don't have an account you can [create one](https://vercel.com/signup?utm_source=turborepo-examples), then enter the following commands:

```
cd my-turborepo
npx turbo login
```

This will authenticate the Turborepo CLI with your [Vercel account](https://vercel.com/docs/concepts/personal-accounts/overview).

Next, you can link your Turborepo to your Remote Cache by running the following command from the root of your Turborepo:

```
npx turbo link
```

## Useful Links

Learn more about the power of Turborepo:

- [Tasks](https://turborepo.com/docs/crafting-your-repository/running-tasks)
- [Caching](https://turborepo.com/docs/crafting-your-repository/caching)
- [Remote Caching](https://turborepo.com/docs/core-concepts/remote-caching)
- [Filtering](https://turborepo.com/docs/crafting-your-repository/running-tasks#using-filters)
- [Configuration Options](https://turborepo.com/docs/reference/configuration)
- [CLI Usage](https://turborepo.com/docs/reference/command-line-reference)
