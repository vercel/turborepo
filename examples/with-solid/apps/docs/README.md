# SolidStart

Everything you need to build a Solid project, powered by [SolidStart](https://docs.solidjs.com/solid-start);

## Creating a project

```bash
# create a new project in the current directory
npm init solid@latest

# create a new project in my-app
npm init solid@latest my-app
```

## Developing

Once you've created a project and installed dependencies with `npm install` (or `pnpm install` or `yarn`), start a development server:

```bash
npm run dev

# or start the server and open the app in a new browser tab
npm run dev -- --open
```

## Building

SolidStart v2 apps are built with [Vite](https://vite.dev/) and deployed with [Nitro](https://nitro.build/) presets, which optimise your project for deployment to different environments.

By default, `npm run build` will generate a Node app that you can run with `npm start`. To use a different preset, configure the `nitro` plugin in your `vite.config.ts`.

## Testing

Tests are written with `vitest`, `@solidjs/testing-library` and `@testing-library/jest-dom` to extend expect with some helpful custom matchers.

To run them, simply start:

```sh
npm test
```

## This project was created with the [Solid CLI](https://solid-cli.netlify.app)
