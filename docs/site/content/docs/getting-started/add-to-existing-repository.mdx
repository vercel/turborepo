---
title: Add to an existing repository
description: Using Turborepo with your existing repository
---

import { PackageManagerTabs, Tabs, Tab } from '#components/tabs';
import { Callout } from '#components/callout';
import { Step, Steps } from '#components/steps';
import { LinkToDocumentation } from '#components/link-to-documentation';

Turborepo can be incrementally adopted in **any repository, single or multi-package**, to speed up the developer and CI workflows of the repository.

After installing `turbo` and configuring your tasks in `turbo.json`, you'll notice how [caching](/docs/crafting-your-repository/caching) helps you run tasks much faster.

## Preparing a single-package workspace

A [single-package workspace](https://vercel.com/docs/vercel-platform/glossary#single-package-workspace) is, for example, what you get after running `npx create-next-app` or `npm create vite`. You don't need to do any extra work for Turborepo to handle your repo so you can jump to the first step below.

To learn more about Turborepo in single-package workspaces, visit [the dedicated guide](/docs/guides/single-package-workspaces).

## Preparing a multi-package workspace (monorepo)

`turbo` is built on top of Workspaces, a feature of the major package managers in the JavaScript ecosystem. This makes it easy to adopt in your existing codebase.

<Callout type="good-to-know">
  If you're finding that `turbo` is having issues like not being able to
  discover packages in your workspace or not following your dependency graph,
  visit our [Structuring a
  repository](/docs/crafting-your-repository/structuring-a-repository) page for
  tips.
</Callout>

Note that you don't have to start running _all_ your tasks for _all_ your
packages using `turbo` right away. You can start with a single task in just a
few packages and incrementally add more tasks and packages as you get more
familiar with Turborepo.

## Adding Turborepo to your repository

<Steps>
<Step>
### Install `turbo`

We recommend you install `turbo` both globally and into your repository's root for the best developer experience.

<PackageManagerTabs>
  <Tab value="pnpm">
      Ensure you have created a `pnpm-workspace.yaml` file before you begin the installation. Failure to have this file will result in an error that says: ` --workspace-root may only be used inside a workspace`.

    ```bash title="Terminal"
    # Global install
    pnpm add turbo --global
    # Install in repository
    pnpm add turbo --save-dev --workspace-root
    ```

  </Tab>
  <Tab value="yarn">

    ```bash title="Terminal"
    # Global install
    yarn global add turbo
    # Install in repository
    yarn add turbo --dev
    ```

  </Tab>
  <Tab value="npm">

    ```bash title="Terminal"
    # Global install
    npm install turbo --global
    # Install in repository
    npm install turbo --save-dev
    ```

  </Tab>
  <Tab value="bun (Beta)">

    ```bash title="Terminal"
    # Global install
    bun install turbo --global
    # Install in repository
    bun install turbo --dev
    ```

  </Tab>
</PackageManagerTabs>

To learn more about why we recommend both installations, visit the [Installation page](/docs/getting-started/installation).

</Step>
<Step>

### Add a `turbo.json` file

In the root of your repository, create a `turbo.json` file.

We'll be using `build` and `check-types` tasks in this guide but you can replace these with other tasks that interest you, like `lint` or `test`.

<Tabs items={['Next.js', 'Vite']} storageKey="framework-preference">
  <Tab value="Next.js">
```json title="./turbo.json"
{
  "$schema": "https://turborepo.com/schema.json",
  "tasks": {
    "build": {
      "dependsOn": ["^build"],
      "outputs": [".next/**", "!.next/cache/**"]
    },
    "check-types": {
      "dependsOn": ["^check-types"]
    },
    "dev": {
      "persistent": true,
      "cache": false
    }
  }
}
```

For more information on configuring your `turbo.json`, see the [Configuration Options](/docs/reference/configuration) documentation.

In your Next.js application, make sure you have a `check-types` script for `turbo` to run.

```diff title="apps/web/package.json"
{
  "scripts": {
+   "check-types": "tsc --noEmit"
  }
}
```

  </Tab>
  <Tab value="Vite">
```json title="./turbo.json"
{
  "$schema": "https://turborepo.com/schema.json",
  "tasks": {
    "build": {
      "outputs": ["dist/**"]
    },
    "check-types": {
      "dependsOn": ["^check-types"]
    },
    "dev": {
      "persistent": true,
      "cache": false
    }
  }
}
```

Some Vite starters ship with a `package.json` that looks like this:

```json title="package.json"
{
  "scripts": {
    "build": "tsc && vite build"
  }
}
```

We recommend splitting these into a `check-types` and `build` script so that `turbo` can run them in parallel.

```json title="apps/web/package.json"
{
  "scripts": {
    "build": "vite build",
    "check-types": "tsc --noEmit"
  }
}
```

  </Tab>
</Tabs>

In a multi-package workspace, you may also want to add a `check-types` script
to one or more of your library packages to see how multiple scripts across
different packages run with one `turbo` command.

</Step>
<Step>
### Edit `.gitignore`

Add `.turbo` to your `.gitignore` file. The `turbo` CLI uses these folders for persisting logs, outputs, and other functionality.

```diff title=".gitignore"
+ .turbo
```

</Step>
<Step>
### Add a `packageManager` field to root `package.json`

Turborepo optimizes your repository using information from your package manager. To declare which package manager you're using, add a [`packageManager`](https://nodejs.org/api/packages.html#packagemanager) field to your root `package.json` if you don't have one already.

<PackageManagerTabs>

<Tab value="pnpm">

```diff title="package.json"
{
+  "packageManager": "pnpm@10.0.0"
}
```

</Tab>

<Tab value="yarn">

```diff title="package.json"
{
+  "packageManager": "yarn@1.22.19"
}
```

</Tab>

<Tab value="npm">

```diff title="package.json"
{
+  "packageManager": "npm@8.5.0"
}
```

</Tab>

<Tab value="bun (Beta)">

```diff title="package.json"
{
+  "packageManager": "bun@1.2.0"
}
```

</Tab>

</PackageManagerTabs>

<Callout type="good-to-know">
  Depending on your repository, you may need to use the
  [`dangerouslyDisablePackageManagerCheck`](/docs/reference/configuration#dangerouslydisablepackagemanagercheck)
  while migrating or in situations where you can't use the `packageManager` key
  yet.
</Callout>

</Step>
<Step>
### Set up package manager workspaces

For [multi-package workspaces](https://vercel.com/docs/glossary#multi-package-workspace), you'll need to configure your package manager to recognize your workspace structure.

The `workspaces` field tells your package manager which directories contain your packages. Common patterns include `apps/*` for applications and `packages/*` for shared libraries.

<Callout type="good-to-know">
  If you're working with a single-package repository, you can skip this step as
  workspaces aren't needed.
</Callout>

<PackageManagerTabs>

  <Tab value="pnpm">
  ```json title="pnpm-workspace.yaml"
 packages:
    - "apps/*"
    - "packages/*"
  ```
  <LinkToDocumentation href="https://pnpm.io/pnpm-workspace_yaml">pnpm workspace documentation</LinkToDocumentation>

  </Tab>

  <Tab value="yarn">
  ```json title="./package.json"
  {
    "workspaces": [
      "apps/*",
      "packages/*"
    ]
  }
  ```

  <LinkToDocumentation href="https://yarnpkg.com/features/workspaces#how-are-workspaces-declared">yarn workspace documentation</LinkToDocumentation>
   </Tab>

  <Tab value="npm">
  ```json title="./package.json"
  {
    "workspaces": [
      "apps/*",
      "packages/*"
    ]
  }
  ```

  <LinkToDocumentation href="https://docs.npmjs.com/cli/v7/using-npm/workspaces#defining-workspaces">npm workspace documentation</LinkToDocumentation>
  </Tab>

  <Tab value="bun (Beta)">
  ```json title="./package.json"
  {
    "workspaces": [
      "apps/*",
      "packages/*"
    ]
  }
  ```

  <LinkToDocumentation href="https://bun.sh/docs/install/workspaces">bun workspace documentation</LinkToDocumentation>
  </Tab>

</PackageManagerTabs>

For more details on how to structure your repository, see [Structuring a Repository](/docs/crafting-your-repository/structuring-a-repository#declaring-directories-for-packages).

</Step>
<Step>
### Run tasks with `turbo`

You can now run the tasks you added to `turbo.json` earlier using Turborepo. Using the example tasks from above:

```bash title="Terminal"
turbo build check-types
```

This runs the `build` and `check-types` tasks at the same time. The dependency graph of your [Workspace](/docs/crafting-your-repository/structuring-a-repository#anatomy-of-a-workspace) will be used to run tasks in the right order.

</Step>
<Step>
**Without making any changes to the code, try running `build` and `check-types` again:**

```bash title="Terminal"
turbo check-types build
```

You should see terminal output like this:

```bash title="Terminal"
Tasks:    2 successful, 2 total
Cached:    2 cached, 2 total
Time:    185ms >>> FULL TURBO
```

Congratulations! **You just built and type checked your code in milliseconds**.

To learn more about how `turbo` makes this possible, check out [the caching documentation](/docs/crafting-your-repository/caching).

</Step>
<Step>

### Begin developing by running `dev` with `turbo`

In a multi-package workspace, you can run `turbo dev` to start the development tasks for all your packages at once.

```bash title="Terminal"
turbo dev
```

You can also [use a filter](/docs/crafting-your-repository/running-tasks#using-filters) to focus on a specific package and its dependencies.

<Callout type="info">
Note that this step doesn't provide much value in a single-package workspace since:

- You don't cache the outputs for a development task.
- There's only one development script so there's nothing to run in parallel.

</Callout>

</Step>

<Step>

## Next steps

You're now up and running with Turborepo! To learn about more ways you can improve your workflow and get the most out of `turbo`, we recommend checking out the following pages:

- [Enabling Remote Caching for development machines](/docs/crafting-your-repository/constructing-ci#enabling-remote-caching)
- [Enabling Remote Caching in CI](/docs/crafting-your-repository/constructing-ci)
- [Handling environment variables](/docs/crafting-your-repository/using-environment-variables)
- [Filtering tasks](/docs/reference/run#--filter-string)

</Step>
</Steps>
