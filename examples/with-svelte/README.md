# Turborepo Svelte starter

This Turborepo starter is maintained by the Turborepo core team on
[GitHub](https://github.com/vercel/turborepo/tree/main/examples/with-svelte).

## Using this example

Run the following command:

```sh
npx create-turbo@latest -e with-svelte
```

To verify that everything works, change into the new project directory:

```shell
pnpm install && pnpm exec turbo run build check-types lint
```

## What's inside?

This Turborepo includes the following packages/apps:

### Apps

- `docs`: a [SvelteKit](https://svelte.dev/docs/kit) app
- `web`: another [SvelteKit](https://svelte.dev/docs/kit) app

### Packages

#### `eslint-config`

ESLint configurations (includes `eslint-plugin-svelte`, `eslint-config-turbo`, and `eslint-config-prettier`).

#### `typescript-config`

A package containing a shared `tsconfig` file.

#### `ui`

A stub Svelte component library shared by both `web` and `docs` applications. The package supports Svelte components and
runes in `.svelte.ts` files, which are not supported in the SvelteKit-generated `tsconfig`.

See the Svelte documentation's [packaging](https://svelte.dev/docs/kit/packaging) page for more information about Svelte
component libraries.

Each package/app is 100% [TypeScript](https://www.typescriptlang.org/).

### Turbo tasks

The following tasks are provided:

- `build`: Building packages
- `check-types`: Running TypeScript checks in Svelte apps and packages.
  - depends on `build`
- `lint`: Running `eslint`.
- `test:unit`: Running unit and component tests.
  - depends on `build`

### Utilities

This Turborepo has tools already set up for you:

- [TypeScript 7](https://www.typescriptlang.org/) for static type checking
- [ESLint 10](https://eslint.org/) for code linting
- [Prettier](https://prettier.io) for code formatting
- [Vitest](https://vitest.dev/) for unit and component testing
