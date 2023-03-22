# Turborepo kitchen sink starter

This is an official starter Turborepo with multiple meta-frameworks all working in harmony and sharing packages.

## What's inside?

This Turborepo includes the following packages and apps:

### Apps

- `apps/api`: an [Express](https://expressjs.com/) server
- `apps/storefront`: a [Next.js](https://nextjs.org/) app
- `apps/admin`: a [Vite](https://vitejs.dev/) single page app
- `apps/blog`: a [Remix](https://remix.run/) blog
- `apps/home`: a [SvelteKit](https://kit.svelte.dev/) landing page

### Packages
- `packages/logger`: isomorphic logger (a small wrapper around console.log)
- `packages/ui`: a dummy React UI library (which contains a single `<CounterButton>` component)
- `packages/scripts`: Jest and ESLint configurations
- `packages/tsconfig`: tsconfig.json;s used throughout the monorepo

Each package and app is 100% [TypeScript](https://www.typescriptlang.org/).

### Utilities

This Turborepo has some additional tools already setup for you:

- [TypeScript](https://www.typescriptlang.org/) for static type checking
- [ESLint](https://eslint.org/) for code linting
- [Jest](https://jestjs.io) test runner for all things JavaScript
- [Prettier](https://prettier.io) for code formatting

## Using this example

Run the following command:

```sh
npx degit vercel/turbo/examples/kitchen-sink kitchen-sink
cd kitchen-sink
pnpm install
git init . && git add . && git commit -m "Init"
```
