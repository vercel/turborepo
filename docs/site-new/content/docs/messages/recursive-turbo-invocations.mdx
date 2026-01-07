---
title: Recursive `turbo` invocations
description: Learn more about errors with recursive scripts and tasks in Turborepo.
---

## Why this error occurred

When a cycle of `turbo` invocations is detected in a [single-package workspace](https://turborepo.com/docs/guides/single-package-workspaces), Turborepo will error to prevent the recursive calls to itself. Typically, this situation occurs for one of two reasons:

### Recursion in scripts and tasks

In a single-package workspace, a script in `package.json` that calls a Turborepo task with the same name causes a loop.

```json title="./package.json"
{
  "scripts": {
    "build": "turbo run build"
  }
}
```

Calling the `build` script calls `turbo run build`. `turbo run build` then calls the `build` script, initiating the loop of recursive calls.

To resolve this, ensure that the name of the script in `package.json` is not the same as the Turborepo task. For example, to fix the snippet above, renaming the script would break the cycle:

```json title="./package.json"
{
  "scripts": {
    "build:app": "turbo run build"
  }
}
```

### Package manager Workspace misconfiguration

A misconfigured workspace can make it appear that a [multi-package workspace](https://vercel.com/docs/vercel-platform/glossary#multi-package-workspace) is a single-package workspace. This causes Turborepo to infer that the repository is of the wrong type, causing it to see the script in `package.json` to be recursive.

Your repo can end up in this state in a few ways, with the most common being that the [packages are not defined according to your package manager](https://turborepo.com/docs/crafting-your-repository/structuring-a-repository#specifying-packages-in-a-monorepo). An npm workspace that is missing the `workspaces` field in `package.json` or a pnpm workspace that is missing a `pnpm-workspace.yaml` file can result in this error message.

Check that your repository is complying with standards for multi-package workspaces and correct any issues.
