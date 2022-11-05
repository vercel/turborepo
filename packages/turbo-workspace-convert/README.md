# `@turbo/workspace-convert`

Easily convert your monorepo between package managers. Currently only supports monorepos using either npm, yarn, or pnpm workspaces.

To get started, open a new shell and run:

## CLI

```sh
npx @turbo/workspace-convert [flags...] [<dir>]
```

Then follow the prompts you see in your terminal.

## Node API

Methods are also available via the Node API:

```ts
import { convertMonorepo, getWorkspaceDetails } from "@turbo/workspace-convert";

// detect the package manager
const project = getWorkspaceDetails({
  workspaceRoot: process.cwd(),
});

// if the package manager is not pnpm, convert to pnpm
if (project.packageManager !== "pnpm") {
  await convertMonorepo({
    root: process.cwd(),
    to: "pnpm",
    options: {
      dry: false,
      install: true,
    },
  });
}
```

See [src/index.ts](src/index.ts) for details

---

For more information about Turborepo, visit [turbo.build/repo](https://turbo.build/repo) and follow us on Twitter ([@turborepo](https://twitter.com/turborepo))!
