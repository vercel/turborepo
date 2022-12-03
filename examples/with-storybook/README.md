# Turborepo starter

This is an official Storybook starter for Turborepo.

Most notably, the `.npmrc` contains a `hoist-pattern[]=!@storybook/*` line to prevent pnpm from trying to hoist Storybook packages.

## What's inside?

This Turborepo uses [pnpm](https://pnpm.io) as a package manager. It includes the following packages/apps:

### Apps and Packages

- `docs`: a [Next.js](https://nextjs.org/) app
- `storybook`: a Vite-based Storybook based app
- `web`: another [Next.js](https://nextjs.org/) app
- `ui`: a stub React component library shared by both `web` and `docs` applications
- `eslint-config-custom`: `eslint` configurations (includes `eslint-config-next` and `eslint-config-prettier`)
- `tsconfig`: `tsconfig.json`s used throughout the monorepo

Each package/app is 100% [TypeScript](https://www.typescriptlang.org/).

### Utilities

This turborepo has some additional tools already setup for you:

- [TypeScript](https://www.typescriptlang.org/) for static type checking
- [ESLint](https://eslint.org/) for code linting
- [Prettier](https://prettier.io) for code formatting

## Using this example

Run the following command:

```sh
npx degit vercel/turbo/examples/with-storybook with-storybook
cd with-storybook
pnpm install
git init . && git add . && git commit -m "Init"
```

### Build

To build all apps and packages, run the following command:

```
cd my-turborepo
pnpm run build
```

### Develop

To develop all apps, packages, and run Storybook, run the following command:

```
cd my-turborepo
pnpm run dev
```
