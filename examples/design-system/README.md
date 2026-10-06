# Turborepo Design System Starter

This is a community-maintained example. If you experience a problem, please submit a pull request with a fix. GitHub Issues will be closed.

A React design system powered by [Turborepo](https://turborepo.dev), [React](https://react.dev), [tsdown](https://tsdown.dev), and [Storybook](https://storybook.js.org/) with Vite. TypeScript, ESLint flat configuration, Prettier, and Changesets are included.

## Using this example

Use Node.js 26.10.0 or newer and pnpm 12.9.1 (pinned in `package.json`):

```sh
npx create-turbo@latest -e design-system
cd design-system
npm install --global pnpm@12.9.1
pnpm install
```

### Useful commands

- `pnpm build`: Build the component library and static Storybook documentation.
- `pnpm dev`: Watch the component library and run Storybook at `http://localhost:6006`.
- `pnpm lint`: Lint source files and configuration with ESLint.
- `pnpm check-types`: Check TypeScript, including unused declarations.
- `pnpm preview-storybook`: Build and serve the static Storybook site.
- `pnpm format`: Format source, configuration, and documentation.
- `pnpm changeset`: Generate a changeset.
- `pnpm clean`: Remove generated builds, task caches, and dependencies.

## Apps and packages

- `apps/docs`: Storybook component documentation.
- `packages/ui`: Publishable React components (`@acme/ui`).
- `packages/typescript-config`: Shared TypeScript configuration.
- `packages/eslint-config`: Shared ESLint flat configurations.

pnpm links internal dependencies using `workspace:*`. Each package declares its own dependencies; use `pnpm add <package> --filter <workspace>` to add one, or `pnpm add -Dw <package>` for root development tooling.

## Turborepo

[Turborepo](https://turborepo.dev) is the build system for coding agents. It runs independent tasks in parallel and caches their outputs. The `build` task builds dependencies first, so Storybook consumes the compiled library. The `check-types` task also builds dependencies before resolving their generated declarations. Development starts with a dependency build before the library watcher and Storybook run together.

`dist/**` and `storybook-static/**` are cached build outputs. Development and preview servers are persistent, uncached tasks. Generated files and dependencies are excluded by `.gitignore`.

## Compilation and components

`packages/ui/tsdown.config.ts` bundles each component into ES modules and CommonJS with declarations. React and its JSX runtime stay external: React is a peer dependency of the library, not bundled into it.

```sh
pnpm build --filter=@acme/ui
```

The button's compiled files are:

```text
packages/ui/dist/
  button.mjs     # ES module
  button.d.mts   # ES module declarations
  button.js      # CommonJS
  button.d.ts    # CommonJS declarations
```

The package exports the appropriate declarations for each module format:

```json
{
  "exports": {
    "./button": {
      "import": {
        "types": "./dist/button.d.mts",
        "default": "./dist/button.mjs"
      },
      "require": {
        "types": "./dist/button.d.ts",
        "default": "./dist/button.js"
      }
    }
  }
}
```

Only `dist` is published. Consumers import components with `import { Button } from "@acme/ui/button"`. To add a component, add its source file to `packages/ui/src`, its entry to `tsdown.config.ts`, and its subpath export to `package.json`.

## Storybook

Storybook uses the React Vite framework, the links addon, and the docs addon. Essential controls and actions are built into current Storybook. The button story uses `tags: ["autodocs"]` to generate documentation and `fn` from `storybook/test` to capture clicks in the actions panel.

Stories use typed Component Story Format:

```tsx
import type { Meta, StoryObj } from "@storybook/react-vite";
import { Button } from "@acme/ui/button";

const meta = {
  component: Button,
  tags: ["autodocs"],
} satisfies Meta<typeof Button>;

export default meta;
type Story = StoryObj<typeof meta>;

export const Default: Story = {
  args: { children: "Hello" },
};
```

Optional MDX files in `apps/docs/stories` can reference these stories using current doc blocks:

```mdx
import { Meta, Canvas, Controls } from "@storybook/addon-docs/blocks";
import * as ButtonStories from "./button.stories";

<Meta of={ButtonStories} />

# Button

<Canvas of={ButtonStories.Primary} />
<Controls />
```

## Versioning and publishing

[Changesets](https://github.com/changesets/changesets) manages package versions and changelogs. Run `pnpm changeset`, select the publishable packages, choose the version bump, and describe the change. Commit the generated file under `.changeset` with your code.

The included release workflow uses pnpm and the pinned Node version. Enable Actions to create pull requests in the repository's Actions settings, and configure an `NPM_TOKEN` repository secret with permission to publish your packages. GitHub supplies the workflow's `GITHUB_TOKEN`. Installing the [Changesets bot](https://github.com/apps/changeset-bot) is optional.

On pushes to `main`, the workflow opens a versioning pull request using `pnpm version-packages`, or publishes the versioned packages with `pnpm release`:

```sh
turbo run build --filter=docs^... && changeset publish
```

This builds the documentation application's dependencies without building Storybook. The documentation and shared configuration packages are private; only `@acme/ui` is publishable.

Before publishing, replace the `@acme` package scope and all its import references with your npm organization, then run `pnpm install`. To publish privately, change the library's `publishConfig.access` and the Changesets `access` setting to `restricted`.
