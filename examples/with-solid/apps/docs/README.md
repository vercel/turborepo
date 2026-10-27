# SolidStart docs app

A [SolidStart v2](https://docs.solidjs.com/solid-start/v2) application with Vitest component tests and shared `@repo/ui` components.

## Setup

Use the Node.js and pnpm versions documented in the [example README](../../README.md). Install dependencies with `pnpm install` from the example root, not this app directory.

Run the following commands from the example root.

## Developing

```sh
pnpm --filter docs dev

# Open the app in a browser
pnpm --filter docs dev --open
```

## Building

```sh
pnpm --filter docs build
pnpm --filter docs start
```

SolidStart uses Vite and the Nitro Vite plugin. The default build produces a Node server at `.output/server/index.mjs`. To deploy elsewhere, configure a Nitro preset in `vite.config.ts`.

## Testing

Tests use Vitest, `@solidjs/testing-library`, and `@testing-library/jest-dom`.

```sh
pnpm --filter docs test
pnpm --filter docs test-watch
pnpm --filter docs test-ui
```
