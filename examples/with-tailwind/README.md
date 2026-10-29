# Turborepo Tailwind CSS starter

This Turborepo starter is maintained by the Turborepo core team.

## Using this example

Use Node.js 26.13.0 or newer and pnpm 12.16.1.

Create a new repository with this example:

```sh
npx create-turbo@latest -e with-tailwind
```

From the generated repository (or this example's directory), install dependencies and start both apps:

```sh
pnpm install
pnpm dev
```

Open `docs` at [http://localhost:3000](http://localhost:3000) and `web` at [http://localhost:3001](http://localhost:3001).

Run `pnpm build`, `pnpm lint`, and `pnpm check-types` from the repository root to build, lint, and type-check all packages.

## What's inside?

This Turborepo includes the following packages/apps:

### Apps and Packages

- `docs`: a [Next.js](https://nextjs.org/) app with [Tailwind CSS](https://tailwindcss.com/)
- `web`: another [Next.js](https://nextjs.org/) app with [Tailwind CSS](https://tailwindcss.com/)
- `ui`: a compiled React component library with [Tailwind CSS](https://tailwindcss.com/) shared by both `web` and `docs` applications
- `@repo/tailwind-config`: shared Tailwind CSS theme and PostCSS configuration
- `@repo/eslint-config`: `eslint` flat configurations (includes `@next/eslint-plugin-next` and `eslint-config-prettier`)
- `@repo/typescript-config`: `tsconfig.json`s used throughout the monorepo

Each package/app is 100% [TypeScript](https://www.typescriptlang.org/).

### Building packages/ui

The `ui` package compiles its components and type declarations with TypeScript and its styles with the Tailwind CSS CLI into `dist`. Both Next.js apps consume these built exports. Turborepo runs `build:components` and `build:styles` before building dependent apps, and builds the shared package before starting their development servers. The `ui` development tasks then watch for component and style changes.

- Share one theme from `packages/tailwind-config/shared-styles.css` across apps and packages.
- Keep component compilation independent of the consuming framework.
- Avoid class-name collisions by using Tailwind CSS v4's `ui:` prefix via `@import "tailwindcss" prefix(ui);` in [packages/ui/src/styles.css](packages/ui/src/styles.css).
- Maintain clear package export boundaries.

Another option is to consume `packages/ui` directly from source without building. Tailwind CSS v4 automatically detects class names in your source files, but it does not scan other packages in `node_modules`. If you use this option, update the package exports to point to source, configure Next.js to transpile `@repo/ui`, and add [`@source` directives](https://tailwindcss.com/docs/functions-and-directives#source-directive) to your apps' CSS entry points. Keep a prefixed CSS entry point for the `ui:` classes, or remove the prefixes when letting the apps generate those styles.

### Utilities

This Turborepo has some additional tools already set up for you:

- [Tailwind CSS](https://tailwindcss.com/) for styles
- [TypeScript](https://www.typescriptlang.org/) for static type checking
- [ESLint](https://eslint.org/) for code linting
- [Prettier](https://prettier.io) for code formatting
