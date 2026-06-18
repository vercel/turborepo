# Turborepo Svelte starter

This Turborepo starter is maintained by the Turborepo core team
on [GitHub](https://github.com/vercel/turborepo/tree/main/examples/with-svelte/packages)
.

## Using this example

Run the following command:

```sh
npx create-turbo@latest -e with-svelte
```

To verify that everything works change in to the new project directory:

```shell
pnpm i && turbo lint build lint:package test:unit
```

## What's inside?

This Turborepo includes the following packages/apps:

### Apps

- `docs`: a [svelte-kit](https://kit.svelte.dev/) app
- `web`: another [svelte-kit](https://kit.svelte.dev/) app

### Packages

#### eslint-config

`eslint` configurations (includes `eslint-plugin-svelte` and `eslint-config-prettier`)

#### typescript-config

A package containing a custom and central `tsconfig` file, that is applied to the applications and the `ui` package. See [NOTES.md](./NOTES.md) for details on the config relationships and the rationale behind individual settings.

#### ui

A stub Svelte component library shared by both `web` and `docs` applications. The package supports Svelte components and
runes in `.svelte.ts` files, which are not supported in the svelte-kit generated tsconfig.

Please refer to the [packaging](https://svelte.dev/docs/kit/packaging) page of the svelte documentation for additional
information about svelte component libraries.

Each package/app is 100% [TypeScript](https://www.typescriptlang.org/).

### Turbo tasks

The following tasks are provided:

- `build`: Building packages
- `check-types`: Running `svelte-check` in Svelte apps and packages.
  - depends on `build`
- `lint`: Running `eslint`.
- `lint:package`: Linting the package.
  - depends on `build`

### Utilities

This Turborepo has tools already setup for you:

- [TypeScript 6](https://www.typescriptlang.org/) for static type checking
- [ESLint 10](https://eslint.org/) for code linting
- [Prettier 10](https://prettier.io) for code formatting
- [Svelte Check](https://github.com/sveltejs/language-tools/tree/master/packages/svelte-check) for the `ui` package as
  well as the `docs` and `web` apps.
- [publint](https://github.com/publint/publint) for linting the `ui` package - not the code itself, i.e. by running
  ```shell
  turbo lint:package --filter @repo/ui
  ```
