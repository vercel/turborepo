# SolidStart web app

A [SolidStart v2](https://docs.solidjs.com/solid-start/v2) application with shared `@repo/ui` components.

## Setup

Use the Node.js and pnpm versions documented in the [example README](../../README.md). Install dependencies with `pnpm install` from the example root, not this app directory.

Run the following commands from the example root.

## Developing

```sh
pnpm --filter web dev

# Open the app in a browser
pnpm --filter web dev --open
```

## Building

```sh
pnpm --filter web build
pnpm --filter web start
```

SolidStart uses Vite and the Nitro Vite plugin. The default build produces a Node server at `.output/server/index.mjs`. To deploy elsewhere, configure a Nitro preset in `vite.config.ts`.
