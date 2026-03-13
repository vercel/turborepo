# Turborepo Svelte starter

This Turborepo starter is maintained by the Turborepo core team
on [GitHub](https://github.com/vercel/turborepo/tree/main/examples/with-svelte/packages)
.

## Using this example

Run the following command:

```sh
npx create-turbo@latest -e with-svelte
```

## What's inside?

This Turborepo includes the following packages/apps:

### Apps

- `docs`: a [svelte-kit](https://kit.svelte.dev/) app
- `web`: another [svelte-kit](https://kit.svelte.dev/) app

### Packages

#### `eslint-config`

`eslint` configurations (includes `eslint-plugin-svelte` and `eslint-config-prettier`)

#### `typescript-config`

A package containing a custom `tsconfig` file.

#### `ui`

A stub Svelte component library shared by both `web` and `docs` applications. The package supports Svelte components and
runes in `.svelte.ts` files, which are not supported in the svelte-kit generated tsconfig.

Please refer to the [packaging](https://svelte.dev/docs/kit/packaging) page of the svelte documentation for additional
information about svelte component libraries.

Each package/app is 100% [TypeScript](https://www.typescriptlang.org/).

### Utilities

This Turborepo has some additional tools already setup for you:

- [TypeScript](https://www.typescriptlang.org/) for static type checking
- [ESLint](https://eslint.org/) for code linting
- [Prettier](https://prettier.io) for code formatting
