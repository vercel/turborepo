---
title: Vitest
description: Learn how to use Vitest in a monorepo.
---

import { Callout } from '#components/callout';
import { File, Folder, Files } from '#components/files';
import { CreateTurboCallout } from './create-turbo-callout.tsx';
import { Tab, Tabs } from '#components/tabs';

[Vitest](https://vitest.dev/) is a test runner from the Vite ecosystem. Integrating it with Turborepo will lead to enormous speed-ups.

[The Vitest documentation](https://vitest.dev/guide/workspace) shows how to create a "Vitest Projects" configuration that runs all tests in the monorepo from one root command, enabling behavior like merged coverage reports out-of-the-box. This feature doesn't follow modern best practices for monorepos, since its designed for compatibility with Jest (whose Workspace feature was built before [package manager Workspaces](/docs/crafting-your-repository/structuring-a-repository)).

<Callout type="warning">
  Vitest has deprecated workspaces in favor of projects. When using projects, individual project vitest configs can't extend the root config anymore since they would inherit the projects configuration. Instead, a separate shared file like `vitest.shared.ts` is needed.
</Callout>

Because of this you have two options, each with their own tradeoffs:

- [Leveraging Turborepo for caching](#leveraging-turborepo-for-caching)
- [Using Vitest's Projects feature](#using-vitests-projects-feature)

### Leveraging Turborepo for caching

To improve on cache hit rates and only run tests with changes, you can choose to configure tasks per-package, splitting up the Vitest command into separate, cacheable scripts in each package. This speed comes with the tradeoff that you'll need to create merged coverage reports yourself.

<Callout>
  For a complete example, run `npx create-turbo@latest --example with-vitest` or
  [visit the example's source
  code](https://github.com/vercel/turborepo/tree/main/examples/with-vitest).
</Callout>

#### Setting up

Let's say we have a simple [package manager Workspace](/docs/crafting-your-repository/structuring-a-repository) that looks like this:

<Files>
  <Folder name="apps" defaultOpen>
    <Folder name="web" defaultOpen>
      <File name="package.json" />
    </Folder>
  </Folder>
  <Folder name="packages" defaultOpen>
    <Folder name="ui" defaultOpen>
      <File name="package.json" />
    </Folder>
  </Folder>
</Files>

Both `apps/web` and `packages/ui` have their own test suites, with `vitest` [installed into the packages that use them](/docs/crafting-your-repository/managing-dependencies#install-dependencies-where-theyre-used). Their `package.json` files include a `test` script that runs Vitest:

```json title="./apps/web/package.json"
{
  "scripts": {
    "test": "vitest run"
  },
  "devDependencies": {
    "vitest": "latest"
  }
}
```

Inside the root `turbo.json`, create a `test` task:

```json title="./turbo.json"
{
  "tasks": {
    "test": {
      "dependsOn": ["transit"]
    },
    "transit": {
      "dependsOn": ["^transit"]
    }
  }
}
```

Now, `turbo run test` can parallelize and cache all of the test suites from each package, only testing code that has changed.

#### Running tests in watch mode

When you run your test suite in CI, it logs results and eventually exits upon completion. This means you can [cache it with Turborepo](/docs/crafting-your-repository/caching). But when you run your tests using Vitest's watch mode during development, the process never exits. This makes a watch task more like a [long-running, development task](/docs/crafting-your-repository/developing-applications).

Because of this difference, we recommend specifying **two separate Turborepo tasks**: one for running your tests, and one for running them in watch mode.

<Callout>
  This strategy below creates two tasks, one for local development and one for
  CI. You could choose to make the `test` task for local development and create
  some `test:ci` task instead.
</Callout>

For example, inside the `package.json` file for each workspace:

```json title="./apps/web/package.json"
{
  "scripts": {
    "test": "vitest run",
    "test:watch": "vitest --watch"
  }
}
```

And, inside the root `turbo.json`:

```json title="./turbo.json"
{
  "tasks": {
    "test": {
      "dependsOn": ["^test"]
    },
    "test:watch": {
      "cache": false,
      "persistent": true
    }
  }
}
```

You can now run your tasks using [global `turbo`](/docs/getting-started/installation#global-installation) as `turbo run test:watch` or from a script in your root `package.json`:

<Tabs items={["Global turbo", "./package.json"]}>
<Tab value="Global turbo">

```bash title="Terminal"
turbo run test

turbo run test:watch
```

</Tab>

<Tab value="./package.json">

```json title="./package.json"
{
  "scripts": {
    "test": "turbo run test",
    "test:watch": "turbo run test:watch"
  }
}
```

</Tab>

</Tabs>

#### Creating merged coverage reports

[Vitest's Projects feature](#using-vitests-projects-feature) creates an out-of-the-box coverage report that merges all of your packages' tests coverage reports. Following the Turborepo strategy, though, you'll have to merge the coverage reports yourself.

<Callout type="info">
  The [`with-vitest`
  example](https://github.com/vercel/turborepo/tree/main/examples/with-vitest)
  shows a complete example that you may adapt for your needs. You can get
  started with it quickly using `npx create-turbo@latest --example with-vitest`.
</Callout>

To do this, you'll follow a few general steps:

1. Run `turbo run test` to create the coverage reports.
2. Merge the coverage reports with [`nyc merge`](https://github.com/istanbuljs/nyc?tab=readme-ov-file#what-about-nyc-merge).
3. Create a report using `nyc report`.

Turborepo tasks to accomplish will look like:

```json title="./turbo.json"
{
  "tasks": {
    "test": {
      "dependsOn": ["^test", "@repo/vitest-config#build"],
      "outputs": ["coverage.json"]
    }
    "merge-json-reports": {
      "inputs": ["coverage/raw/**"],
      "outputs": ["coverage/merged/**"]
    },
    "report": {
      "dependsOn": ["merge-json-reports"],
      "inputs": ["coverage/merge"],
      "outputs": ["coverage/report/**"]
    },
  }
}
```

With this in place, run `turbo test && turbo report` to create a merged coverage report.

<Callout type="info">
  The [`with-vitest`
  example](https://github.com/vercel/turborepo/tree/main/examples/with-vitest)
  shows a complete example that you may adapt for your needs. You can get
  started with it quickly using `npx create-turbo@latest --example with-vitest`.
</Callout>

### Using Vitest's Projects feature

The Vitest Projects feature doesn't follow the same model as a [package manager Workspace](/docs/crafting-your-repository/structuring-a-repository). Instead, it uses a root script that then reaches out into each package in the repository to handle the tests in that respective package.

In this model, there aren't package boundaries, from a modern JavaScript ecosystem perspective. This means you can't rely on Turborepo's caching, since Turborepo leans on those package boundaries.

Because of this, you'll need to use [Root Tasks](/docs/crafting-your-repository/configuring-tasks#registering-root-tasks) if you want to run the tests using Turborepo. Once you've configured [a Vitest Projects setup](https://vitest.dev/guide/workspace), create the Root Tasks for Turborepo:

```json title="./turbo.json"
{
  "tasks": {
    "//#test": {
      "outputs": ["coverage/**"]
    },
    "//#test:watch": {
      "cache": false,
      "persistent": true
    }
  }
}
```

**Notably, the file inputs for a Root Task include all packages by default, so any change in any package will result in a cache miss.** While this does make for a simplified configuration to create merged coverage reports, you'll be missing out on opportunities to cache repeated work.

### Using a hybrid approach

You can combine the benefits of both approaches by implementing a hybrid solution. This approach unifies local development using Vitest's Projects feature while preserving Turborepo's caching in CI. This comes at the tradeoff of slightly more configuration and a mixed task running model in the repository.

First, create a shared configuration package since individual projects can't extend the root config when using projects. Create a new package for your shared Vitest configuration:

```json title="./packages/vitest-config/package.json"
{
  "name": "@repo/vitest-config",
  "version": "0.0.0",
  "main": "dist/index.js",
  "types": "dist/index.d.ts",
  "scripts": {
    "build": "tsc",
    "dev": "tsc --watch"
  },
  "dependencies": {
    "vitest": "latest"
  },
  "devDependencies": {
    "@repo/typescript-config": "workspace:*",
    "typescript": "latest"
  }
}
```

```json title="./packages/vitest-config/tsconfig.json"
{
  "extends": "@repo/typescript-config/base.json",
  "compilerOptions": {
    "outDir": "dist",
    "rootDir": "src"
  },
  "include": ["src"],
  "exclude": ["dist", "node_modules"]
}
```

```ts title="./packages/vitest-config/src/index.ts"
export const sharedConfig = {
  test: {
    globals: true,
    environment: 'jsdom',
    setupFiles: ['./src/test/setup.ts'],
    // Other shared configuration
  }
};
```

Then, create your root Vitest configuration using projects:

```ts title="./vitest.config.ts"
import { defineConfig } from 'vitest/config';
import { sharedConfig } from '@repo/vitest-config';

export default defineConfig({
  ...sharedConfig,
  projects: [
    {
      name: 'packages',
      root: './packages/*',
      test: {
        ...sharedConfig.test,
        // Project-specific configuration
      }
    }
  ]
});
```

In this setup, your packages maintain their individual Vitest configurations that import the shared config. First, install the shared config package:

```json title="./packages/ui/package.json"
{
  "scripts": {
    "test": "vitest run",
    "test:watch": "vitest --watch"
  },
  "devDependencies": {
    "@repo/vitest-config": "workspace:*",
    "vitest": "latest"
  }
}
```

Then create the Vitest configuration:

```ts title="./packages/ui/vitest.config.ts"
import { defineConfig } from 'vitest/config';
import { sharedConfig } from '@repo/vitest-config';

export default defineConfig({
  ...sharedConfig,
  test: {
    ...sharedConfig.test,
    // Package-specific overrides if needed
  }
});
```

Make sure to update your `turbo.json` to include the new configuration package in the dependency graph:

```json title="./turbo.json"
{
  "tasks": {
    "build": {
      "dependsOn": ["^build"],
      "outputs": ["dist/**"]
    },
    "test": {
      "dependsOn": ["^test", "@repo/vitest-config#build"]
    },
    "test:watch": {
      "cache": false,
      "persistent": true
    }
  }
}
```

While your root `package.json` includes scripts for running tests globally:

```json title="./package.json"
{
  "scripts": {
    "test:projects": "vitest run",
    "test:projects:watch": "vitest --watch"
  }
}
```

This configuration allows developers to run `pnpm test:projects` or `pnpm test:projects:watch` at the root for a seamless local development experience using Vitest projects, while CI continues to use `turbo run test` to leverage package-level caching. **You'll still need to handle merged coverage reports manually as described in the previous section**.
