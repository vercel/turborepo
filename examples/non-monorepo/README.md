# Turborepo non-monorepo starter

This Turborepo starter is maintained by the Turborepo core team.

## Using this example

Use Node.js 26 and npm 12.2.0. Run the following command:

```sh
npx create-turbo@latest -e non-monorepo --package-manager npm
```

For an existing checkout of this example, install dependencies from its directory:

```sh
npm ci
```

## What's inside?

This Turborepo uses a single, non-monorepo project (in this case, a single Next.js application).

TypeScript 7 checks the application both during builds and in the `check-types` task.
ESLint uses Babel to parse TypeScript without requiring the older TypeScript
JavaScript compiler API, with Next.js Core Web Vitals and React Hooks rules.

### Tasks

There are several Turborepo tasks already set up for you to use.

#### Build the application

```sh
npx turbo build
```

#### Lint source code

```sh
npx turbo lint
```

#### Type check source code

```sh
npx turbo check-types
```

This generates Next.js route types before running the TypeScript compiler.

#### Run the application's development server

```sh
npx turbo dev
```

## Useful Links

Learn more about the power of Turborepo:

- [Tasks](https://turborepo.dev/docs/crafting-your-repository/running-tasks)
- [Caching](https://turborepo.dev/docs/crafting-your-repository/caching)
- [Remote Caching](https://turborepo.dev/docs/core-concepts/remote-caching)
- [Filtering](https://turborepo.dev/docs/crafting-your-repository/running-tasks#using-filters)
- [Configuration Options](https://turborepo.dev/docs/reference/configuration)
- [CLI Usage](https://turborepo.dev/docs/reference/command-line-reference)
