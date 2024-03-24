# Turborepo starter with shadcn/ui

![Static Badge](https://img.shields.io/badge/shadcn%2Fui-latest-blue?link=https%3A%2F%2Fgithub.com%2Fshadcn%2Fui)

This is Turborepo starter with shadcn/ui pre-configured.

> **Note**
> This example uses `pnpm` as package manager.

## Using this example

Clone the repository:

```sh
git clone git@github.com:Parthvsquare/turbo-next-vite-schadcn.git
```

Install dependencies:

```sh
cd turbo-next-vite-schadcn
pnpm install
```

### Add ui components

Use the pre-made script:

```sh
pnpm ui:add <component-name>
```

> This works just like the add command in the `shadcn/ui` CLI.

## What's inside?

This Turborepo includes the following packages/apps:

### Apps and Packages

- `admin`: a [Next.js](https://nextjs.org/) app
- `super-admin`: [Vite + React](https://vitejs.dev/) and [Ant design](https://ant.design/) app and using [tanstack router](https://tanstack.com/router/latest) and [jotai](https://jotai.org/) for state management
- `web`: another [Next.js](https://nextjs.org/) app
- `ui`: a stub React component library shared by both `web` and `docs` applications (🚀 powered by **shadcn/ui**)
- `eslint-config-custom`: `eslint` configurations (includes `eslint-config-next` and `eslint-config-prettier`)
- `tsconfig`: `tsconfig.json`s used throughout the monorepo

Each package/app is 100% [TypeScript](https://www.typescriptlang.org/).

### Utilities

This Turborepo has some additional tools already setup for you:

- [TypeScript](https://www.typescriptlang.org/) for static type checking
- [ESLint](https://eslint.org/) for code linting
- [Prettier](https://prettier.io) for code formatting

### Build

To build all apps and packages, run the following command:

```sh
cd turbo-next-vite-schadcn
pnpm build
```

### Develop

To develop all apps and packages, run the following command:

```sh
cd turbo-next-vite-schadcn
pnpm dev
```

To develop a specific app or package, run the following command:

```sh
cd turbo-next-vite-schadcn
make run-<app-name>
```

### Remote Caching

Turborepo can use a technique known as [Remote Caching](https://turbo.build/repo/docs/core-concepts/remote-caching) to share cache artifacts across machines, enabling you to share build caches with your team and CI/CD pipelines.

By default, Turborepo will cache locally. To enable Remote Caching you will need an account with Vercel. If you don't have an account you can [create one](https://vercel.com/signup), then enter the following commands:

```
cd turbo-next-vite-schadcn
npx turbo login
```

This will authenticate the Turborepo CLI with your [Vercel account](https://vercel.com/docs/concepts/personal-accounts/overview).

Next, you can link your Turborepo to your Remote Cache by running the following command from the root of your Turborepo:

```sh
npx turbo link
```

## Useful Links

Learn more about the power of Turborepo:

- [Tasks](https://turbo.build/repo/docs/core-concepts/monorepos/running-tasks)
- [Caching](https://turbo.build/repo/docs/core-concepts/caching)
- [Remote Caching](https://turbo.build/repo/docs/core-concepts/remote-caching)
- [Filtering](https://turbo.build/repo/docs/core-concepts/monorepos/filtering)
- [Configuration Options](https://turbo.build/repo/docs/reference/configuration)
- [CLI Usage](https://turbo.build/repo/docs/reference/command-line-reference)

Learn more about shadcn/ui:

- [Documentation](https://ui.shadcn.com/docs)
