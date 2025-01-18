# Turborepo starter

This is a community-maintained example. If you experience a problem, please submit a pull request with a fix. GitHub Issues will be closed.

## Using this example

Run the following command:

```sh
npx create-turbo@latest -e with-angular
```

## What's inside?

This Turborepo includes the following pckages/apps:

### Apps and Packages

- `docs` an [angular](https://angular.dev/) app
- `web` another [angular](https://angular.dev/) app
- `ui` a stub Angular component library shared by both `web` and `docs` application
- `eslint-config`: `eslint` configurations (based on [@angular-eslint/eslint-plugin](https://github.com/angular-eslint/angular-eslint#readme))

Each package/app is 100% [TypeScript](https://www.typescriptlang.org/).

### Utilities

This Turborepo has some additional tools already setup for you:

- [TypeScript](https://www.typescriptlang.org/) for static type checking
- [ESLint](https://eslint.org/) for code linting
- [Prettier](https://prettier.io/) for code formatting
