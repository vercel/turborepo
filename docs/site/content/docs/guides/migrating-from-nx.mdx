---
title: Migrating from Nx
description: Learn how to migrate to Turborepo from Nx.
---

import { PackageManagerTabs, Tabs, Tab } from '#components/tabs';

This guide will help you migrate an existing Nx repository to Turborepo.

- Explore key concepts by [migrating from an Nx starter to Turborepo](#migration-steps)
- Considerations for [more complex migration scenarios](#advanced-migration-considerations)

## Why switch?

There are many reasons why you may be choosing to migrate from Nx to Turborepo. Below, we've listed the most common motivations that developers have referenced for their migrations.

### Using ecosystem standards

Turborepo's goal is to be lightweight, leaning on your repository as the source of truth. An example of this is Turborepo being [built on top of JavaScript package manager workspaces](/docs/crafting-your-repository/structuring-a-repository) for it's JavaScript/TypeScript support.

By contrast, Nx uses layers of plugins, dependencies, and other Nx-specific code to infer information about your repository. While these plugins can provide a layer of functionality and are optional, Nx users looking to migrate often cite removing Nx-specific code from their codebase as a key motivation for their change.

### Greater control of source code

Nx’s philosophy involves wrapping your code with layers of plugins, dependencies, and Nx-specific code. While these layers of code are optional, they provide a great deal of Nx's value and are recommended by Nx, so most Nx repos have them. When migrating to Turborepo, many developers explain that these layers tend to create a layer of obfuscation that abstracts their repository away from their control, causing issues.

Turborepo chooses to let you handle your tooling on your own terms, configuring (or not configuring) any of your tooling as you please.

### Less configuration for your repository manager

Migrating to Turborepo will likely require deleting previous configuration that you had for Nx and replacing it with less configuration for Turborepo, since Turborepo will automatically infer your repository's needs. For example, here are the tool-specific configurations you'll find in the equivalent starters for Turborepo and Nx [used below](#migration-steps).

<Tabs items={["Turborepo", "Nx"]}>

<Tab value="Turborepo">

```json title="turbo.json"
{
  "$schema": "/schema.json",
  "ui": "tui",
  "tasks": {
    "build": {
      "dependsOn": ["^build"],
      "inputs": ["$TURBO_DEFAULT$", ".env*"],
      "outputs": [".next/**", "!.next/cache/**"]
    },
    "lint": {
      "dependsOn": ["^lint"]
    },
    "check-types": {
      "dependsOn": ["^check-types"]
    },
    "dev": {
      "cache": false,
      "persistent": true
    }
  }
}
```

</Tab>

<Tab value="Nx">

```json title="nx.json"
{
  "$schema": "./node_modules/nx/schemas/nx-schema.json",
  "namedInputs": {
    "default": ["{projectRoot}/**/*", "sharedGlobals"],
    "production": [
      "default",
      "!{projectRoot}/.eslintrc.json",
      "!{projectRoot}/eslint.config.cjs",
      "!{projectRoot}/**/?(*.)+(spec|test).[jt]s?(x)?(.snap)",
      "!{projectRoot}/tsconfig.spec.json",
      "!{projectRoot}/jest.config.[jt]s",
      "!{projectRoot}/src/test-setup.[jt]s",
      "!{projectRoot}/test-setup.[jt]s"
    ],
    "sharedGlobals": ["{workspaceRoot}/.github/workflows/ci.yml"]
  },
  "nxCloudId": "6789ec521d90a2165398f39a",
  "plugins": [
    {
      "plugin": "@nx/next/plugin",
      "options": {
        "startTargetName": "start",
        "buildTargetName": "build",
        "devTargetName": "dev",
        "serveStaticTargetName": "serve-static"
      }
    },
    {
      "plugin": "@nx/playwright/plugin",
      "options": {
        "targetName": "e2e"
      }
    },
    {
      "plugin": "@nx/eslint/plugin",
      "options": {
        "targetName": "lint"
      }
    },
    {
      "plugin": "@nx/jest/plugin",
      "options": {
        "targetName": "test"
      }
    }
  ],
  "targetDefaults": {
    "e2e-ci--**/*": {
      "dependsOn": ["^build"]
    }
  },
  "generators": {
    "@nx/next": {
      "application": {
        "style": "tailwind",
        "linter": "eslint"
      }
    }
  }
}
```

```json title="project.json"
{
  "name": "starter",
  "$schema": "../../node_modules/nx/schemas/project-schema.json",
  "sourceRoot": "apps/starter",
  "projectType": "application",
  "tags": [],
  "// targets": "to see all targets run: nx show project starter --web",
  "targets": {}
}
```

</Tab>

</Tabs>

## Migration steps

Our goal for this migration is to get a working Turborepo task as quickly as possible, so that you can adopt Turborepo features incrementally. We’ll start by using the Nx scaffolder to create a repository with a Next.js app.

```bash title="Terminal"
npx create-nx-workspace --preset=next --ci=skip --e2eTestRunner=none --style=tailwind --nextAppDir=true --nextSrcDir=false --packageManager=pnpm --appName=starter
```

### Step 1: Update .gitignore

Turborepo uses the .turbo directory to hold local caches and other information about your repository. For this reason, it should be added to your `.gitignore`.

```txt title=".gitignore"
.turbo
```

### Step 2: Add a workspace definition

Turborepo is built on top of package manager workspaces, a JavaScript ecosystem standard. Add the directory paths to the workspace that will contain packages.

<PackageManagerTabs>

<Tab value="pnpm">

```yml title="pnpm-workspace.yaml"
packages:
  - apps/*
```

</Tab>

<Tab value="yarn">

```json title="package.json"
{
  "workspaces": ["apps/*"]
}
```

</Tab>

<Tab value="npm">

```json title="package.json"
{
  "workspaces": ["apps/*"]
}
```

</Tab>

<Tab value="bun (Beta)">

```json title="package.json"
{
  "workspaces": ["apps/*"]
}
```

</Tab>

</PackageManagerTabs>

### Step 3: Add a package.json to the application

Rather than adding additional configuration files like `project.json`, Turborepo uses the standard `package.json` file.

Add a `package.json` to the `starter` application. Create a `package.json` at `./apps/starter/package.json` that contains a `dev` and `build` script.

```json title="./apps/starter/package.json"
{
  "name": "starter",
  "scripts": {
    "dev": "next dev",
    "build": "next build"
  }
}
```

### Step 4: Remove Nx plugin

Remove the Nx plugin from ./apps/starter/next.config.js. The example file below doesn’t have configuration, though your existing Next.js application may need some.

```js title="./apps/starter/next.config.js"
/** @type {import('next').NextConfig} */
const nextConfig = {};

module.exports = nextConfig;
```

### Step 5: Add the `packageManager` field

The root package.json needs to have the `packageManager` field. This ensures developers in the repository use the correct package manager, and that Turborepo can optimize your package graph based on your lockfile.

<PackageManagerTabs>

<Tab value="pnpm">

```json title="./package.json"
{
  "packageManager": "pnpm@9.0.0"
}
```

</Tab>

<Tab value="yarn">

```json title="./package.json"
{
  "packageManager": "yarn@1.22.19"
}
```

</Tab>

<Tab value="npm">

```json title="./package.json"
{
  "packageManager": "npm@10.0.0"
}
```

</Tab>

<Tab value="bun (Beta)">

```json title="./package.json"
{
  "packageManager": "bun@1.2.0"
}
```

</Tab>

</PackageManagerTabs>

### Step 6: Run you package manager's install command

Update your lockfile by running your installation command.

<PackageManagerTabs>

<Tab value="pnpm">

```bash title="Terminal"
pnpm install
```

</Tab>

<Tab value="yarn">

```bash title="Terminal"
yarn install
```

</Tab>

<Tab value="npm">

```bash title="Terminal"
npm install
```

</Tab>

<Tab value="bun (Beta)">

```bash title="Terminal"
bun install
```

</Tab>

</PackageManagerTabs>

Once you've done this, you should see a lockfile diff, indicating that the package has been added to the package manager's workspace.

### Step 7: Install Turborepo

Add Turborepo to the root `package.json` of the workspace.

<PackageManagerTabs>

<Tab value="pnpm">

```bash title="Terminal"
pnpm add turbo --save-dev --workspace-root
```

</Tab>

<Tab value="yarn">

```bash title="Terminal"
 yarn add turbo --save-dev --ignore-workspace-root-check
```

</Tab>

<Tab value="npm">

```bash title="Terminal"
npm install turbo --save-dev
```

</Tab>

<Tab value="bun (Beta)">

```bash title="Terminal"
bun install turbo --dev
```

</Tab>

</PackageManagerTabs>

You can also optionally install `turbo` globally for added convenience when working with Turborepo.

<PackageManagerTabs>

<Tab value="pnpm">

```bash title="Terminal"
pnpm add turbo --global
```

</Tab>

<Tab value="yarn">

```bash title="Terminal"
yarn global add turbo
```

</Tab>

<Tab value="npm">

```bash title="Terminal"
npm install turbo --global
```

</Tab>

<Tab value="bun (Beta)">

```bash title="Terminal"
bun install turbo --global
```

</Tab>

</PackageManagerTabs>

### Step 8: Add a `turbo.json`

Create a `turbo.json` at the root to register your tasks and describe their task dependencies.

```json title="./turbo.json"
{
  "tasks": {
    "build": {
      "dependsOn": ["^build"],
      "outputs": [".next/**", "!.next/cache/**"]
    },
    "dev": {
      "cache": false,
      "persistent": true
    }
  }
}
```

### Step 9: Run `turbo build`

Build the application with Turborepo. Using global `turbo`, this would be `turbo build`. You can also run the command through your package manager:

<PackageManagerTabs>

<Tab value="pnpm">

```bash title="Terminal"
pnpm exec turbo build
```

</Tab>

<Tab value="yarn">

```bash title="Terminal"
 yarn dlx turbo build
```

</Tab>

<Tab value="npm">

```bash title="Terminal"
npx turbo run build
```

</Tab>

<Tab value="bun (Beta)">

```bash title="Terminal"
bunx turbo run build
```

</Tab>

</PackageManagerTabs>

### Step 10: Enable Remote Caching (optional)

By default, Turborepo will connect to the free-to-use Vercel Remote Cache when you run:

```bash title="Terminal"
turbo login
turbo link
```

You may also configure a self-hosted Remote Cache.

## Advanced migration considerations

While the migration guide above is a good starting point, the breadth of possibilities and capabilities of monorepos means that its difficult to create generalized instructions for all cases. Below, we’ve listed some common next steps that you may be thinking about.

### Migrate complex monorepos incrementally

We encourage incremental migration, meaning you will have both of Nx and Turborepo in your repository at the same time. Make sure to spend time understanding how your Nx task graph is constructed. Splitting up the task graph may include strategies like:

- **Migrating one task at a time**: Changing `nx run lint` to `turbo run lint`
- **Migrating one package/project at a time**: Changing `nx run-many lint test --projects=web` to `turbo run lint test --filter=web`
- **Double-running some of your tasks**: To ensure stability, you may choose to run `turbo run lint` **and** `nx run lint` while you're still getting comfortable and building certainty in the early phases of your migration.

### Installing dependencies where they're used

Turborepo recommends [installing packages where they're used](/docs/crafting-your-repository/managing-dependencies#best-practices-for-dependency-installation) to improve cache hit ratios, help dependency pruning capability, and clarify for developers which dependencies are meant for which packages. This is different from the Nx strategy, where all dependencies are installed at the root of the repository, making all dependencies available to all packages in the workspace.

Historically, Nx has recommended installing all dependencies in the root of the repository, making all dependencies available to all packages in the Workspace. If you followed this guidance, we highly recommend that you move dependencies to the `package.json`'s for packages and applications that need them. [Visit our documentation on managing dependencies](/docs/crafting-your-repository/managing-dependencies) to learn more.

### Creating shared packages

You’ll follow roughly the same set of steps as above to add a package to your package manager’s workspace.

1. Ensure the package’s directory is included in the workspace definition (like `./packages/*` ).
2. Add a `package.json` to the package with the scripts it needs to run.
3. Check task dependencies in `turbo.json` to make sure your dependency graph meets your requirements.

### Multi-language monorepos

Turborepo natively supports JavaScript and TypeScript, with secondary support for any other languages you’d like to use. [Visit the Multi-Language support documentation](/docs/guides/multi-language) to learn more.

## Configuration equivalents

Configuration found in `nx.json` can be mapped to `turbo.json` using the tables below.

<Callout type="info">
  The majorify of globs for capturing files are the same between Nx and
  Turborepo. See [our file glob specification](/docs/reference/globs) for
  details and edge cases.
</Callout>

### Global configuration

| Nx                         | Turborepo                                                                |
| -------------------------- | ------------------------------------------------------------------------ |
| `sharedGlobals`            | [`globalDependencies`](/docs/reference/configuration#globaldependencies) |
| `sharedGlobals.env`        | [`globalEnv`](/docs/reference/configuration#globalenv)                   |
| `sharedGlobals.namedInput` | [`globalDependencies`](/docs/reference/configuration#globaldependencies) |
| `cacheDirectory`           | [`cacheDir`](/docs/reference/configuration#cachedir)                     |

### Task configuration

| Nx              | Turborepo                                                      |
| --------------- | -------------------------------------------------------------- |
| `inputs` files  | [`tasks[task].inputs`](/docs/reference/configuration#inputs)   |
| `inputs.env`    | [`tasks[task].env`](/docs/reference/configuration#env)         |
| `outputs` files | [`tasks[task].outputs`](/docs/reference/configuration#outputs) |
| `cache`         | [`tasks[task].cache`](/docs/reference/configuration#cache)     |

### CLI equivalents

| Nx               | Turborepo                                                               |
| ---------------- | ----------------------------------------------------------------------- |
| `nx generate`    | [`turbo generate`](/docs/reference/generate)                            |
| `nx run`         | [`turbo run`](/docs/reference/run)                                      |
| `nx run-many`    | [`turbo run`](/docs/reference/run)                                      |
| `nx reset`       | [`--force`](/docs/reference/run#--force)                                |
| `--parallel`     | [`--concurrency`](/docs/reference/run#--concurrency-number--percentage) |
| `--nxBail`       | [`--continue`](/docs/reference/run#--continueoption)                    |
| `--projects`     | [`--filter`](/docs/reference/run#--filter-string)                       |
| `--graph`        | [`--graph`](/docs/reference/run#--graph-file-name)                      |
| `--output-style` | [`--log-order`](/docs/reference/run#--log-order-option)                 |
| `--no-cloud`     | [`--cache`](/docs/reference/run#--cache-options)                        |
| `--verbose`      | [`--verbosity`](/docs/reference/run#--verbosity)                        |
