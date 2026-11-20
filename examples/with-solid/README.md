# Solid-Turborepo Starter

A modern Turborepo starter for building SolidJS applications with a complete development setup. This starter comes equipped with the pnpm package manager, SolidStart v2 on Vite and Nitro for optimized builds, Tailwind CSS v4 for styling and ESLint for code quality.

---

## Table of Contents

- [Overview](#overview)
- [Getting Started](#getting-started)
- [What's Inside?](#whats-inside)
- [Build & Develop](#build--develop)
- [Remote Caching](#remote-caching)
- [Useful Links](#useful-links)
- [License](#license)

## Overview

This starter project leverages Turborepo to manage a monorepo structure that combines SolidStart applications with shared component libraries and configuration packages. It provides a robust setup for modern web development.

## Getting Started

This example requires Node.js 26.12.1 or newer and pnpm 12.15.0, as pinned in `package.json`.

To bootstrap your new turborepo using this starter, run:

```sh
npx create-turbo@latest --example with-solid --package-manager pnpm
```

When working from a checkout of this example, install dependencies from its root:

```sh
cd with-solid
pnpm install
```

## What's inside?

This Turborepo includes the following packages/apps:

### Apps and Packages

- `docs`: a [SolidStart](https://docs.solidjs.com/solid-start) app with [Vitest](https://vitest.dev/) component tests
- `web`: a [SolidStart](https://docs.solidjs.com/solid-start) app
- `@repo/ui`: a stub Solid component library shared by both Solid applications
- `@repo/eslint-config`: shared [ESLint](https://eslint.org/) flat configurations
- `@repo/tailwind-config`: [Tailwind](https://tailwindcss.com/) v4 configurations

Both applications are built with [SolidStart v2](https://docs.solidjs.com/solid-start/v2), which runs on [Vite](https://vite.dev/) and the [Nitro](https://nitro.build/) Vite plugin.

## Build & Develop

Run the following commands from the example root.

### Build

To build all apps and packages:

```sh
pnpm run build
```

### Develop

To develop all apps and packages:

```sh
pnpm run dev
```

### Test

To run the Vitest component tests:

```sh
pnpm run test
```

### Lint and type-check

```sh
pnpm run lint
pnpm run check-types
```

## Remote Caching

> [!TIP]
> Vercel Remote Cache is free for all plans. Get started today at [vercel.com](https://vercel.com/signup?utm_source=remote-cache-sdk&utm_campaign=free_remote_cache).

Turborepo can use [Remote Caching](https://turborepo.dev/docs/core-concepts/remote-caching) to share cache artifacts across machines, enabling you to share build caches with your team and CI/CD pipelines.

By default, Turborepo will cache locally. To enable Remote Caching you will need an account with Vercel. If you don't have an account you can [create one](https://vercel.com/signup?utm_source=turborepo-examples), then run the following command from the example root:

```sh
pnpm exec turbo login
```

This will authenticate the Turborepo CLI with your [Vercel account](https://vercel.com/docs/concepts/personal-accounts/overview).

Next, link your Turborepo to your Remote Cache:

```sh
pnpm exec turbo link
```

## Useful Links

Learn more about the power of Turborepo:

- [Tasks](https://turborepo.dev/docs/crafting-your-repository/running-tasks)
- [Caching](https://turborepo.dev/docs/crafting-your-repository/caching)
- [Remote Caching](https://turborepo.dev/docs/core-concepts/remote-caching)
- [Filtering](https://turborepo.dev/docs/crafting-your-repository/running-tasks#using-filters)
- [Configuration Options](https://turborepo.dev/docs/reference/configuration)
- [CLI Usage](https://turborepo.dev/docs/reference/command-line-reference)

## License

MIT
