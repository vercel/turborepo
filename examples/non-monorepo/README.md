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

- [Tasks](https://turbo.build/repo/docs/core-concepts/monorepos/running-tasks)
- [Caching](https://turbo.build/repo/docs/core-concepts/caching)
- [Remote Caching](https://turbo.build/repo/docs/core-concepts/remote-caching)
- [Filtering](https://turbo.build/repo/docs/core-concepts/monorepos/filtering)
- [Configuration Options](https://turbo.build/repo/docs/reference/configuration)
- [CLI Usage](https://turbo.build/repo/docs/reference/command-line-reference)
