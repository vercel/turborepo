---
title: Developing applications
description: Learn how to develop applications in your repository.
---

import { Tabs, Tab } from '#components/tabs';
import { LinkToDocumentation } from '#components/link-to-documentation';

Developing applications in a monorepo unlocks powerful workflows, enabling you to make atomic commits to source control with easy access to code.

Most development tasks are long-running tasks that watch for changes to your code. Turborepo enhances this experience with a powerful terminal UI and other capabilities like:

- [Configuration for `dev` tasks](#configuring-development-tasks)
- [Interacting with tasks](#interacting-with-tasks)
- [Watch Mode](#watch-mode)
- [Running setup scripts](#running-setup-tasks-before-dev)
- [Filtering tasks to run a subset of your packages](#running-a-specific-application)

## Configuring development tasks

Defining a development task in `turbo.json` tells Turborepo that you'll be running a long-lived task. This is useful for things like running a development server, running tests, or building your application.

To register a `dev` task, add it to your `turbo.json` with two properties:

```json title="./turbo.json"
{
  "tasks": {
    "dev": {
      "cache": false,
      "persistent": true
    }
  }
}
```

- **"cache": false**: Tells Turborepo to not attempt to cache the results of the task. Since this is a development task, you're likely to be making frequent changes to your code, so caching the results is not useful.
- **"persistent": true**: Tells Turborepo to keep the task running until you stop it. This key serves as a signal for your terminal UI to treat the task as long-running and interactive. Additionally, it prevents you from accidentally depending on a task that will not exit.

You can now run your `dev` task to start your development scripts in parallel:

```bash title="Terminal"
turbo dev
```

### Running setup tasks before `dev`

You may also want to run scripts that set up your development environment or pre-build packages. You can make sure those tasks run before the `dev` task with `dependsOn`:

```json title="./turbo.json"
{
  "tasks": {
    "dev": {
      "cache": false,
      "persistent": true,
      "dependsOn": ["//#dev:setup"]
    },
    "//#dev:setup": {
      "outputs": [".codegen/**"]
    }
  }
}
```

In this example, we're using a [Root Task](/docs/crafting-your-repository/configuring-tasks#registering-root-tasks) but you can use the same idea for [arbitrary tasks in packages](/docs/crafting-your-repository/configuring-tasks#depending-on-a-specific-task-in-a-specific-package).

### Running a specific application

The `--filter` flag allows you to pick a subset of your [Package Graph](/docs/core-concepts/package-and-task-graph#package-graph) so you can run your `dev` task for a specific application and its dependencies:

```bash title="Terminal"
turbo dev --filter=web
```

## Using the terminal UI

Turborepo's terminal UI enables a number of features that create a highly interactive experience around your tasks.

### Customizing your view

You can quickly adjust the UI to your needs using keybinds.

| Keybind | Action                                                            |
| ------- | ----------------------------------------------------------------- |
| `m`     | Toggle popup listing keybinds                                     |
| `↑`/`↓` | Select the next/previous task in the task list                    |
| `j`/`k` | Select the next/previous task in the task list                    |
| `p`     | Toggle selection pinning for selected task                        |
| `h`     | Toggle visibility of the task list                                |
| `c`     | When logs are highlighted, copy selection to the system clipboard |
| `u`/`d` | Scroll logs `u`p and `d`own                                       |

### Interacting with tasks

Some of your tools may allow you to type input into them. Examples of this include Drizzle ORM's interactive migrations or Jest's filtering and re-running of test suites.

You can interact with tasks that are [marked as interactive](/docs/reference/configuration#interactive) to give them input.

| Keybind  | Action            |
| -------- | ----------------- |
| `i`      | Begin interacting |
| `Ctrl+z` | Stop interacting  |

## Watch Mode

Many tools have a built-in watcher, like [`tsc --watch`](https://www.typescriptlang.org/docs/handbook/compiler-options.html#compiler-options),
that will respond to changes in your source code. However, some don't.

`turbo watch` adds a dependency-aware watcher to any tool. Changes to source code will follow [the Task Graph](/docs/core-concepts/package-and-task-graph#task-graph) that you've described in `turbo.json`, just like all your other tasks.

For example, using a package structure like [`create-turbo`](/docs/reference/create-turbo) with the following tasks and scripts:

<Tabs items={["turbo.json", "packages/ui", "apps/web"]}>
<Tab value="turbo.json">

```json title="turbo.json"
{
  "tasks": {
    "dev": {
      "persistent": true,
      "cache": false
    },
    "lint": {
      "dependsOn": ["^lint"]
    }
  }
}
```

</Tab>

<Tab value="packages/ui">

```json title="package.json"
{
  "name": "@repo/ui"
  "scripts": {
    "dev": "tsc --watch",
    "lint": "eslint ."
  }
}
```

</Tab>

<Tab value="apps/web">

```json title="package.json"
{
  "name": "web"
  "scripts": {
    "dev": "next dev",
    "lint": "eslint ."
  },
  "dependencies": {
      "@repo/ui": "workspace:*"
    }
}
```

</Tab>
</Tabs>

When you run `turbo watch dev lint`, you'll see the `lint` scripts are re-run whenever you make source code changes, despite ESLint not having a built-in watcher. `turbo watch` is also aware of internal dependencies, so a code change in `@repo/ui` will re-run the task in both `@repo/ui` and `web`.

The Next.js development server in `web` and the TypeScript Compiler's built-in watcher in `@repo/ui` will continue to work as usual, since they are marked with `persistent`.

For more information, [visit the `turbo watch` reference](/docs/reference/watch).

## Limitations

### Teardown tasks

In some cases, you may want to run a script when the `dev` task is stopped. Turborepo is unable to run those teardown scripts when exiting because `turbo` exits when your `dev` tasks exit.

Instead, create a `turbo dev:teardown` script that you run separately after you've exited your primary `turbo dev` task.

## Next steps

Once you have a version of your application that you'd like to deploy, it's time to learn how to configure environment variables in Turborepo.
