# `Turborepo` Vite starter

This is an official starter Turborepo.

## What's inside?

This Turborepo includes the following packages and apps:

### Apps and Packages

- `api`: a firebase clould function using express, built with rollup
- `web`: a vanilla [vite](https://vitejs.dev) ts app
- `config`: eslint shared config
- `shared`: shared util lib that you share code with various apps
- `tsconfig`: `tsconfig.json`s used throughout the monorepo

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
npx degit vercel/turborepo/examples/with-firebase with-firebase
cd with-firebase
yarn
git init . && git add . && git commit -m "Init"
```
