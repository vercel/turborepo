---
title: Upgrading
description: Learn how to upgrade `turbo` to get the latest improvements to your repository.
---

import { PackageManagerTabs, Tab } from '#components/tabs';
import { Steps, Step } from '#components/steps';
import { Callout } from '#components/callout';

## Upgrading to 2.0

<Steps>
<Step>

### Update `turbo.json`

Get started upgrading from 1.x to 2.0 by running:

<PackageManagerTabs>

<Tab value="pnpm">

```bash title="Terminal"
pnpm dlx @turbo/codemod migrate
```

</Tab>

<Tab value="yarn">

```bash title="Terminal"
yarn dlx @turbo/codemod migrate
```

</Tab>

<Tab value="npm">

```bash title="Terminal"
npx @turbo/codemod migrate
```

</Tab>

<Tab value="bun (Beta)">

```bash title="Terminal"
bunx @turbo/codemod migrate
```

</Tab>

</PackageManagerTabs>

This will update your `turbo.json`(s) for many of the breaking changes from 1.x to 2.0.

Additionally, a `name` field will be added to any `package.json` in the Workspace that doesn't have one.

<Callout type="good-to-know">
  You may also manually run each codemod individually. Visit [the codemods
  page](/docs/reference/turbo-codemod#turborepo-2x) for more information.
</Callout>

</Step>

<Step>

### Add a `packageManager` field to root `package.json`

[The `packageManager` field](https://nodejs.org/api/packages.html#packagemanager) is a convention from the Node.js ecosystem that defines which package manager is expected to be used in the Workspace.

Turborepo 2.0 requires that your Workspace define this field as a way to improve the stability and behavioral predictability of your codebase. If you do not have one already, add this field to your root `package.json`:

<PackageManagerTabs>

<Tab value="pnpm">

```diff title="./package.json"
{
+ "packageManager": "pnpm@9.2.0"
}
```

</Tab>

<Tab value="yarn">

```diff title="./package.json"
{
+ "packageManager": "yarn@1.22.19"
}
```

</Tab>

<Tab value="npm">

```diff title="./package.json"
{
+ "packageManager": "npm@10.8.1"
}
```

</Tab>

<Tab value="bun (Beta)">

```diff title="./package.json"
{
+ "packageManager": "bun@1.2.0"
}
```

</Tab>
</PackageManagerTabs>

</Step>
<Step>

### Update `eslint-config-turbo`

[`eslint-config-turbo`](/docs/reference/eslint-config-turbo) helps identify environment variables that need to be added to the [`env`](/docs/reference/configuration#env) key for caching. If you're using it, make sure you update it to match your major version.

</Step>

<Step>

### Update `turbo run` commands

Turborepo 2.0 includes behavioral and correctness improvements with behavior of `turbo run` commands. Listed below is the summary of changes, which may or may not have an affect on your codebase:

- Strict Mode for environment variables is now the default, moving from Loose Mode ([PR](https://github.com/vercel/turborepo/pull/8182))
  - If it appears that the scripts in your tasks are missing environment variables, you can opt back out of this behavior using [the `--env-mode` option](/docs/reference/run#--env-mode-option) on a per-command basis to incrementally migrate. We encourage you to update [the `env` key](/docs/reference/configuration#env) in your task to account for all of its environment variables so you can drop the `--env-mode` option as soon as possible.
  - If you'd like to set the default for the repository back to Loose Mode, you can do so [using the `envMode` configuration](/docs/reference/configuration#envmode).
- Workspace root directory is now an implicit dependency of all packages ([PR](https://github.com/vercel/turborepo/pull/8202))
  - The repository should have as little code in the root as possible, since changes to the root can affect all tasks in your repository. Additionally, if you're using [Internal Packages](/docs/core-concepts/internal-packages) in the Workspace root, changes to those dependencies will also cause cache misses for all tasks. In both cases, consider moving the code out of the root and [into a package](/docs/crafting-your-repository/structuring-a-repository).
- `--ignore` removed in favor of `--filter` and graph correctness changes below ([PR](https://github.com/vercel/turborepo/pull/8201))
- Removed `--scope` flag (deprecated since 1.2) ([PR](https://github.com/vercel/turborepo/pull/7970))
- `engines` field in root `package.json` is now used in hashing ([PR](https://github.com/vercel/turborepo/pull/8173))
- `--filter` no longer infers namespaces for package names ([PR](https://github.com/vercel/turborepo/pull/8137))
- `--filter` now errors when no package names or directories are matched ([PR](https://github.com/vercel/turborepo/pull/8142))
- `--only` restricts task dependencies instead of package dependencies ([PR](https://github.com/vercel/turborepo/pull/8163))

</Step>
</Steps>
