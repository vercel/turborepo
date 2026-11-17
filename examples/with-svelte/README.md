# Turborepo Svelte starter

This Turborepo starter is maintained by the Turborepo core team on
[GitHub](https://github.com/vercel/turborepo/tree/main/examples/with-svelte).

## Requirements

- [Node.js](https://nodejs.org/) `24.x`
- [pnpm](https://pnpm.io/) `12.18.0`, enabled through [Corepack](https://nodejs.org/api/corepack.html)

Corepack reads the `packageManager` field in the root `package.json` and runs the pinned pnpm version. Enable it once:

```shell
corepack enable
```

## Using this example

Create a new project from this example:

```sh
npx create-turbo@latest -e with-svelte
```

Change into the new project directory, install dependencies, and start the dev servers:

```shell
pnpm install
pnpm dev
```

The dev servers use these default URLs:

| App         | URL                   |
| ----------- | --------------------- |
| `web`       | http://localhost:5173 |
| `docs`      | http://localhost:5174 |
| `storybook` | http://localhost:6006 |

### Commands and tasks

Run these from the repository root. The build, check, lint, test, and dev commands map to the corresponding Turbo tasks:

| Command             | Turbo task    | Description                                             |
| ------------------- | ------------- | ------------------------------------------------------- |
| `pnpm dev`          | `dev`         | Start the web, docs, and Storybook dev servers          |
| `pnpm build`        | `build`       | Build every app and package                             |
| `pnpm check-types`  | `check-types` | Run Svelte and TypeScript checks after building         |
| `pnpm lint`         | `lint`        | Check formatting, then run ESLint                       |
| `pnpm test:unit`    | `test:unit`   | Run unit and browser tests after building               |
| `pnpm format`       | —             | Format the workspace with Prettier                      |
| `pnpm format:check` | —             | Check formatting; `lint` runs this as `//#format:check` |

`pnpm format` and `pnpm format:check` invoke Prettier directly. The same format check is registered as the root
`//#format:check` task so `lint` can run it once for the whole workspace.

Task behavior:

- `build` builds each package in dependency order and caches `.svelte-kit`, `.turbo`, `.vercel`, `dist`, and
  `storybook-static` outputs.
- `check-types` depends on `build` and uses `svelte-check` instead of `tsc` alone. It checks component scripts,
  templates, props, bindings, event handlers, and the SvelteKit-generated `$app` types. Because each package uses its
  own `tsconfig.json`, shared `@repo/ui` components are checked both where they are defined and where the apps consume
  them.
- `lint` runs the root `//#format:check` task first, then ESLint in each package.
- `dev` runs the SvelteKit and Storybook dev servers as persistent, uncached tasks.
- `test:unit` depends on `build` and runs the Vitest browser and server test projects.

To run a command for one workspace package, use a filter:

```shell
pnpm --filter web dev
pnpm --filter docs build
pnpm --filter storybook dev
pnpm --filter @repo/ui test:unit
```

To verify the workspace before committing:

```shell
pnpm build && pnpm check-types && pnpm lint && pnpm test:unit
```

`test:unit` runs Vitest in browser mode with Playwright as the browser provider; there is no separate Playwright Test
suite. If Chromium is not installed, run `pnpm --filter @repo/ui exec playwright install chromium` once. The
`playwright.config.ts` files in `apps/web` and `apps/docs` are optional scaffolding for adding Playwright e2e tests
later.

## What's inside?

This Turborepo includes the following packages/apps:

### Apps

- `docs`: a [SvelteKit](https://svelte.dev/docs/kit) app that renders Markdown and Svelte components with
  [mdsvex](https://mdsvex.pngwn.io/)
- `web`: another [SvelteKit](https://svelte.dev/docs/kit) app that uses plain Svelte components
- `storybook`: a [Storybook](https://storybook.js.org/) app for developing and documenting the shared UI components

The apps share dependency versions and SvelteKit configuration. `docs` additionally enables mdsvex so its routes can use
`.svx` files.

The Svelte packages are versioned through the `sveltejs` catalog in `pnpm-workspace.yaml`. This keeps `svelte`,
`svelte-check`, the SvelteKit packages, and the Svelte plugins in lockstep while leaving the rest of each application's
tooling explicit in its `package.json`. When upgrading a Svelte package, change its catalog entry and run `pnpm install`
to refresh the lockfile.

Other shared tooling versions are kept aligned directly in each `package.json`. Update the apps and `@repo/ui` together.

### Packages

#### `eslint-config`

ESLint configurations (includes `eslint-plugin-svelte`, `eslint-config-turbo`, and `eslint-config-prettier`).

#### `typescript-config`

A package containing a shared `tsconfig` file.

The apps and the `ui` package extend both this shared config and `$app/tsconfig`:

```json
{
  "extends": ["@repo/typescript-config/svelte.json", "$app/tsconfig"]
}
```

`$app/tsconfig` is not a package dependency. It is generated by `svelte-kit sync` at `node_modules/$app/tsconfig.json`, and
TypeScript resolves it as a module when processing `extends`. The generated config provides SvelteKit's recommended
compiler options, path aliases, `rootDirs`, and the generated `$app/types` route types. Because it lives in
`node_modules`, it is recreated by `svelte-kit sync` and should not be edited by hand. Entries later in the `extends`
array override earlier ones, so `$app/tsconfig` takes precedence over the shared config, while the package's own
`compilerOptions` override both.

#### `ui`

A stub Svelte component library shared by the `web`, `docs`, and `storybook` applications. The package supports Svelte
components and runes in `.svelte.ts` files, which are not supported in the SvelteKit-generated `tsconfig`.

`@repo/ui` declares `svelte` as a peer dependency (`^5.57.3`) rather than a regular dependency. The apps provide the
Svelte runtime, so the package reuses the app's Svelte instance instead of installing a second copy. The caret range
accepts compatible Svelte 5 releases without causing peer conflicts, while the `devDependency` (`catalog:sveltejs`) is
used for local type checking and tests.

See the Svelte documentation's [packaging](https://svelte.dev/docs/kit/packaging) page for more information about Svelte
component libraries.

Each package/app is 100% [TypeScript](https://www.typescriptlang.org/).

### Utilities

This Turborepo has tools already set up for you:

- [TypeScript 6](https://www.typescriptlang.org/) for static type checking
- [svelte-check](https://www.npmjs.com/package/svelte-check) for Svelte component and template diagnostics
- [ESLint 10](https://eslint.org/) for code linting
- [Prettier](https://prettier.io) for code formatting
- [Vitest](https://vitest.dev/) for unit and component testing
- [Storybook](https://storybook.js.org/) for developing and documenting UI components in isolation

### Dependency versions

Shared tooling versions are kept aligned across the apps and `@repo/ui`, so the workspace resolves a single Vite,
Vitest, Playwright, ESLint, and TypeScript dependency tree. This also means a single browser download for the component
tests.

`@types/node` is pinned to the Node.js 24 line to match the runtime required by `engines.node` (`24.x`). Type
declarations should not be ahead of the runtime major, otherwise code can type-check against APIs that do not exist on
Node.js 24. The exact patch version does not need to match the runtime; the major version does.

`vitest` and `@vitest/browser-playwright` are kept on the same version because the browser matchers, including
`toBeInTheDocument()`, are typed across both packages. Upgrading one without the other produces type errors in the
browser tests. Keep both aligned across the apps and `@repo/ui`.

## SvelteKit configuration

SvelteKit 3 no longer supports `svelte.config.js` or `svelte.config.ts`. Configuration is passed to the `sveltekit()`
Vite plugin instead. `apps/web` and `apps/docs` define their preprocessor and adapter in `vite.config.ts`:

```ts
import adapter from '@sveltejs/adapter-auto';
import { sveltekit } from '@sveltejs/kit/vite';
import { vitePreprocess } from '@sveltejs/vite-plugin-svelte';
import { defineConfig } from 'vite';

export default defineConfig({
  plugins: [
    sveltekit({
      preprocess: vitePreprocess(),
      adapter: adapter()
    })
  ]
});
```

SvelteKit throws `config_file_unsupported` when either file is present. `packages/ui` is a component library, not a
SvelteKit app, so it passes the preprocessor directly to the plain Svelte plugin in `vite.config.js`.

See [Migrating to SvelteKit 3](https://svelte.dev/docs/kit/migrating-to-sveltekit-3#Configuration-svelte.config.js-is-no-longer-supported)
and the [`@sveltejs/kit/vite` configuration reference](https://svelte.dev/docs/kit/@sveltejs-kit-vite) for details.

## mdsvex

`apps/docs` renders its root route from `src/routes/+page.svx`. The file is Markdown with an embedded
`<CounterView />` from `@repo/ui`, and mdsvex compiles it to a Svelte component:

```svx
<script lang="ts">
  import { CounterView } from '@repo/ui';
</script>

# Docs

<CounterView />
```

To enable `.svx` routes, the docs app registers the extension and preprocessor in `vite.config.ts`:

```ts
sveltekit({
  extensions: ['.svelte', '.svx'],
  preprocess: [vitePreprocess(), mdsvex()],
  adapter: adapter()
});
```

Because `svelte-check` only discovers `.svelte` files, `.svx` routes are not type-checked directly. `+page.svx.d.ts`
declares the module for TypeScript, and `+layout.svelte` keeps a Svelte file available to the checker. `apps/web` does
not enable mdsvex and remains a plain SvelteKit app.

## Storybook

`apps/storybook` develops and documents the components exported by `@repo/ui`. It uses `@storybook/svelte-vite` with
`@storybook/addon-svelte-csf`, which lets stories use Svelte syntax. Stories live in
`apps/storybook/src/stories`; the example story covers the `CounterView` component from `@repo/ui`:

- `apps/storybook/src/stories/CounterView.stories.svelte`

Storybook's Vite builder loads the app's `vite.config.ts`, so the Svelte plugin must be registered there:

```ts
import { svelte } from '@sveltejs/vite-plugin-svelte';
import { defineConfig } from 'vite';

export default defineConfig({
  plugins: [svelte()]
});
```

Run the Storybook app by itself with:

```shell
# Start Storybook at http://localhost:6006
pnpm --filter storybook dev

# Write the static site to apps/storybook/storybook-static
pnpm --filter storybook build
```

Turborepo caches `storybook-static/**`, and Git and Prettier ignore it.

## TypeScript 7 note

This example uses TypeScript 6. SvelteKit 3's `svelte-kit sync` relies on the TypeScript compiler API
(`ts.sys` and `ts.readConfigFile`) to read and validate `tsconfig.json`. TypeScript 7 is the native compiler and its
`typescript` entry point does not expose that JavaScript API, so sync fails with:

```text
Cannot read properties of undefined (reading 'readFile')
```

`svelte-check` 4.x also only declares support for TypeScript 5 and 6. Keep TypeScript 6 until SvelteKit and
`svelte-check` support TypeScript 7. See the
[TypeScript 7 announcement](https://devblogs.microsoft.com/typescript/announcing-typescript-7-0/#running-side-by-side-with-typescript-6.0)
for the side-by-side compatibility strategy.
