# Turborepo non-monorepo starter

This Turborepo starter is maintained by the Turborepo core team.

## Using this example

Run the following command:

```sh
npx create-turbo@latest -e non-monorepo
```

## What's inside?

This Turborepo uses a single, non-monorepo project (in this case, a single Next.js application).

### Tasks

There are several Turborepo tasks already set up for you to use.

#### Build the application

```
pnpm turbo build
```

#### Lint source code

```
pnpm turbo lint
```

#### Type check source code

```
pnpm turbo check-types
```

#### Run the application's development server

```
pnpm turbo dev
```

## Useful Links

Learn more about the power of Turborepo:

- [Tasks](https://turborepo.dev/docs/crafting-your-repository/running-tasks)
- [Caching](https://turborepo.dev/docs/crafting-your-repository/caching)
- [Remote Caching](https://turborepo.dev/docs/core-concepts/remote-caching)
- [Filtering](https://turborepo.dev/docs/crafting-your-repository/running-tasks#using-filters)
- [Configuration Options](https://turborepo.dev/docs/reference/configuration)
- [CLI Usage](https://turborepo.dev/docs/reference/command-line-reference)
