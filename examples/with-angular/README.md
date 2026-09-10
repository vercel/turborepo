# Turborepo Angular starter

This is a community-maintained example. If you experience a problem, please submit a pull request with a fix. GitHub Issues will be closed.

## Using this example

Create a new copy of the example:

```sh
npx create-turbo@latest -e with-angular
```

Then install dependencies and run an application:

```sh
pnpm install
pnpm --filter docs start
```

Run `pnpm --filter web start` in another terminal to start the second application.

## What's inside?

This Turborepo includes the following packages and apps:

### Apps and packages

- `docs`: an [Angular](https://angular.dev/) app
- `web`: another [Angular](https://angular.dev/) app
- `ui`: an Angular component library shared by both applications
- `eslint-config`: shared ESLint flat configurations based on [angular-eslint](https://github.com/angular-eslint/angular-eslint#readme)

Each package and app is written in [TypeScript](https://www.typescriptlang.org/).

### Utilities

This Turborepo includes:

- [TypeScript](https://www.typescriptlang.org/) for static type checking
- [ESLint](https://eslint.org/) for code linting
- [Prettier](https://prettier.io/) for code formatting
