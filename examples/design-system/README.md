# Turborepo Design System Starter

This is a community-maintained example. If you experience a problem, please submit a pull request with a fix. GitHub Issues will be closed.

A React design system with [Turborepo](https://turborepo.dev), [React](https://react.dev), [tsup](https://tsup.egoist.dev), and [Storybook](https://storybook.js.org). TypeScript, ESLint flat configs, Prettier, and Changesets are preconfigured.

## Using this example

Install Node.js 26.13.0 or newer and pnpm 12.18.0, then create the project:

```sh
npx create-turbo@latest -e design-system
cd design-system
pnpm install
```

### Useful commands

- `pnpm build` — Build the component library and static Storybook documentation.
- `pnpm dev` — Build dependencies, then watch the library and run Storybook at `http://localhost:6006`.
- `pnpm lint` — Lint the library, stories, and tooling configurations.
- `pnpm check-types` — Type-check the library and documentation app.
- `pnpm preview-storybook` — Build and serve the static Storybook site.
- `pnpm changeset` — Generate a changeset for publishable packages.
- `pnpm clean` — Remove generated builds, task caches, and dependencies.

## Apps and packages

- `apps/docs`: Storybook documentation using the React Vite framework.
- `packages/ui`: Publishable React components in `@acme/ui`.
- `packages/typescript-config`: Shared TypeScript configurations.
- `packages/eslint-config`: Shared ESLint flat configurations.

pnpm links local packages through `workspace:*` dependencies. Add dependencies to the package that imports them; use `pnpm add -w -D <package>` only for root tooling.

## Compilation

`pnpm build` uses Turborepo to build dependencies before their consumers and cache build outputs. The UI library uses `packages/ui/tsup.config.ts` to produce ES modules and CommonJS, then runs TypeScript 7's `tsc --emitDeclarationOnly` to generate declarations in `packages/ui/dist`.

Each component has an explicit export in `packages/ui/package.json`:

```json
{
  "exports": {
    "./button": {
      "types": "./dist/button.d.ts",
      "import": "./dist/button.mjs",
      "require": "./dist/button.js"
    }
  }
}
```

Only `dist` is published. React is a peer dependency so consumers provide their own React instance. When adding components, add their entry points to the tsup configuration and their exports to `package.json`.

ESLint uses Babel to parse TypeScript syntax without depending on the TypeScript compiler API. TypeScript's `noUnusedLocals` and `noUnusedParameters` checks cover unused declarations, and the compiler checks undefined names. Run both `pnpm lint` and `pnpm check-types` for complete checks.

## Storybook

Storybook 10 uses an ESM configuration in `apps/docs/.storybook/main.ts`. It discovers stories in `apps/docs/stories`, and the docs addon provides automatic component documentation. Controls and actions are built into Storybook; the links addon supports navigation between stories. To add MDX documentation, add a `../stories/**/*.mdx` pattern to the `stories` configuration.

The included button story uses typed Component Story Format and an explicit action spy:

```tsx
import type { Meta, StoryObj } from "@storybook/react-vite";
import { fn } from "storybook/test";
import { Button } from "@acme/ui/button";

const meta = {
  component: Button,
  tags: ["autodocs"],
  args: { onClick: fn() },
} satisfies Meta<typeof Button>;

export default meta;
type Story = StoryObj<typeof meta>;

export const Primary: Story = {
  args: { children: "Hello", type: "button" },
};
```

Stories import the same built package exports as consumers. Turborepo builds the library before starting Storybook and before checking the documentation app's types.

## Versioning and publishing

[Changesets](https://github.com/changesets/changesets) manages package versions, changelogs, and npm publishing. The documentation app and shared configurations are private; only the UI library is published.

Before publishing:

1. Replace the `@acme` scope in package names and imports with a scope you own, then run `pnpm install`.
2. Configure `NPM_TOKEN` in your GitHub repository secrets with permission to publish to that scope. The release workflow uses GitHub's automatically supplied `GITHUB_TOKEN` to create release pull requests.
3. Ensure `main` is your release branch. The supplied workflow assumes this example is the repository root.

Run `pnpm changeset`, select the packages to release, choose the version bump, and write a summary. Commit the generated file in `.changeset` with your changes.

On pushes to `main`, the release workflow installs with pnpm and opens or updates a version pull request using `pnpm version-packages`. After that pull request is merged, it runs:

```sh
pnpm release
```

The release script builds the documentation app's dependencies (the publishable library) with `turbo run build --filter=docs^...`, then runs `changeset publish`.

For restricted npm packages, change `access` to `restricted` in both `.changeset/config.json` and the library's `publishConfig`.
