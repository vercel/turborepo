---
title: ESLint
description: Learn how to use ESLint in a monorepo.
---

import { PackageManagerTabs, Tabs, Tab } from '#components/tabs';
import { Callout } from '#components/callout';
import { Files, Folder, File } from '#components/files';
import { CreateTurboCallout } from './create-turbo-callout.tsx';

ESLint is a static analysis tool for quickly finding and fixing problems in your JavaScript code.

<CreateTurboCallout />

In this guide, we'll cover:

- [ESLint v9 with Flat Configuration](#eslint-v9-flat-configs)
- [ESLint v8 with legacy configuration](#eslint-v8-legacy)
- [How to set up a `lint` task (applies to both versions)](#setting-up-a-lint-task)

We will share configurations across the monorepo's Workspace, ensuring configuration is consistent across packages and composable to maintain high cache hit ratios.

## ESLint v9 (Flat Configs)

Using ESLint v9's Flat Configs, we will end up with a file structure like this:

<Files>
  <Folder name="apps" defaultOpen>
    <Folder name="docs" defaultOpen>
     <File name="package.json" />
     <File name="eslint.config.js" green />
    </Folder>

    <Folder name="web" defaultOpen>
     <File name="package.json" />
     <File name="eslint.config.js" green />
    </Folder>

  </Folder>

  <Folder name="packages" defaultOpen>
    <Folder name="eslint-config" defaultOpen>
     <File name="base.js" green />
     <File name="next.js" green />
     <File name="react-internal.js" green />
     <File name="package.json" />
    </Folder>

    <Folder name="ui" defaultOpen>
     <File name="eslint.config.js" green />
     <File name="package.json" />
    </Folder>

  </Folder>
</Files>

This structure includes:

- A package called `@repo/eslint-config` in `./packages/eslint-config` that holds all ESLint configuration
- Two applications, each with their own `eslint.config.js`
- A `ui` package that also has its own `eslint.config.js`

### About the configuration package

The `@repo/eslint-config` package has three configuration files, `base.js`, `next.js`, and `react-internal.js`. They are [exported from `package.json`](https://github.com/vercel/turborepo/blob/main/examples/basic/packages/eslint-config/package.json#L6) so that they can be used by other packages, according to needs. Examples of the configurations can be found [in the Turborepo GitHub repository](https://github.com/vercel/turborepo/tree/main/examples/basic/packages/eslint-config) and are available in `npx create-turbo@latest`.

Notably, the `next.js` and `react-internal.js` configurations use the `base.js` configuration for consistency, extending it with more configuration for their respective requirements. Additionally, notice that [the `package.json` for `eslint-config`](https://github.com/vercel/turborepo/blob/main/examples/basic/packages/eslint-config/package.json) has all of the ESLint dependencies for the repository. This is useful, since it means we don't need to re-specify the dependencies in the packages that import `@repo/eslint-config`.

### Using the configuration package

In our `web` app, we first need to add `@repo/eslint-config` as a dependency.

<PackageManagerTabs>

  <Tab value="pnpm">
```jsonc title="./apps/web/package.json"
{
  "devDependencies": {
    "@repo/eslint-config": "workspace:*"
  }
}
```
  </Tab>

  <Tab value="yarn">
```jsonc title="./apps/web/package.json"
{
  "devDependencies": {
    "@repo/eslint-config": "*"
  }
}
```
  </Tab>

  <Tab value="npm">
```jsonc title="./apps/web/package.json"
{
  "devDependencies": {
    "@repo/eslint-config": "*"
  }
}
```
  </Tab>

  <Tab value="bun (Beta)">
```jsonc title="./apps/web/package.json"
{
  "devDependencies": {
    "@repo/eslint-config": "workspace:*"
  }
}
```
  </Tab>
</PackageManagerTabs>

We can then import the configuration like this:

```js title="./apps/web/eslint.config.js"
import { nextJsConfig } from '@repo/eslint-config/next-js';

/** @type {import("eslint").Linter.Config} */
export default nextJsConfig;
```

Additionally, you can add configuration specific to the package like this:

```js title="./apps/web/eslint.config.js"
import { nextJsConfig } from '@repo/eslint-config/next-js';

/** @type {import("eslint").Linter.Config} */
export default [
  ...nextJsConfig,
  // Other configurations
];
```

## ESLint v8 (Legacy)

<Callout type="warn">
  ESLint v8 is end-of-life as of October 5, 2024. We encourage you to upgrade to
  ESLint v9 or later. This documentation is here to help with existing projects
  that have not yet upgraded.
</Callout>

Using legacy configuration from ESLint v8 and lower, we will end up with a file structure like this:

<Files>
  <Folder name="apps" defaultOpen>
    <Folder name="docs" defaultOpen>
     <File name="package.json" />
     <File name=".eslintrc.js" green />
    </Folder>

    <Folder name="web" defaultOpen>
     <File name="package.json" />
     <File name=".eslintrc.js" green />
    </Folder>

  </Folder>

  <Folder name="packages" defaultOpen>
    <Folder name="eslint-config" defaultOpen>
     <File name="base.js" green />
     <File name="next.js" green />
     <File name="react-internal.js" green />
     <File name="package.json" />
    </Folder>

    <Folder name="ui" defaultOpen>
     <File name=".eslintrc.js" green />
     <File name="package.json" />
    </Folder>

  </Folder>
</Files>

There's a package called `@repo/eslint-config`, and two applications, each with their own `.eslintrc.js`.

### The `@repo/eslint-config` package

The `@repo/eslint-config` file contains two files, `next.js`, and `library.js`. These are two different ESLint configurations, which we can use in different packages, depending on our needs.

A configuration for Next.js may look like this:

```js title="./packages/eslint-config/next.js"
/* Custom ESLint configuration for use with Next.js apps. */
module.exports = {
  extends: [
    'eslint-config-turbo',
    'eslint-config-next',
    // ...your other ESLint configurations
  ].map(require.resolve),
  // ...your other configuration
};
```

The `package.json` looks like this:

```json title="./packages/eslint-config/package.json"
{
  "name": "@repo/eslint-config",
  "version": "0.0.0",
  "private": true,
  "devDependencies": {
    "eslint": "^8",
    "eslint-config-turbo": "latest",
    "eslint-config-next": "latest"
  }
}
```

Note that the ESLint dependencies are all listed here. This is useful, since it means we don't need to re-specify the dependencies inside the apps which import `@repo/eslint-config`.

### How to use the `@repo/eslint-config` package

In our `web` app, we first need to add `@repo/eslint-config` as a dependency.

<PackageManagerTabs>

  <Tab value="pnpm">
```jsonc title="./apps/web/package.json"
{
  "dependencies": {
    "@repo/eslint-config": "workspace:*"
  }
}
```
  </Tab>
  <Tab value="yarn">
```jsonc title="./apps/web/package.json"
{
  "dependencies": {
    "@repo/eslint-config": "*"
  }
}
```
  </Tab>

  <Tab value="npm">
```jsonc title="./apps/web/package.json"
{
  "dependencies": {
    "@repo/eslint-config": "*"
  }
}
```
  </Tab>

  <Tab value="bun (Beta)">
```jsonc title="./apps/web/package.json"
{
  "dependencies": {
    "@repo/eslint-config": "workspace:*"
  }
}
```
  </Tab>
</PackageManagerTabs>

We can then import the config like this:

```js title="./apps/web/.eslintrc.js"
module.exports = {
  root: true,
  extends: ['@repo/eslint-config/next.js'],
};
```

By adding `@repo/eslint-config/next.js` to our `extends` array, we're telling ESLint to look for a package called `@repo/eslint-config`, and reference the file `next.js`.

## Setting up a `lint` task

The `package.json` for each package where you'd like to run ESLint should look like this:

```json title="./packages/*/package.json"
{
  "scripts": {
    "lint": "eslint ."
  }
}
```

With your scripts prepared, you can then create your Turborepo task:

```bash title="./turbo.json"
{
  "tasks": {
    "lint": {}
  }
}
```

You can now run `turbo lint` with [global `turbo`](/docs/getting-started/installation#global-installation) or create a script in your root `package.json`:

```json title="./package.json"
{
  "scripts": {
    "lint": "turbo run lint"
  }
}
```
