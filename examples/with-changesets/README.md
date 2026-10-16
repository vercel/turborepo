# Turborepo Design System starter with Changesets

This is a community-maintained example. If you experience a problem, please submit a pull request with a fix. GitHub Issues will be closed.

## Using this example

Run the following command:

```sh
npx create-turbo@latest -e with-changesets
```

Use Node.js 26.11.1 or newer and pnpm 12.11.2, as pinned in `package.json`. Install dependencies with `pnpm install`.

## What's inside?

This Turborepo includes the following:

### Apps and Packages

- `@acme/docs`: A placeholder documentation site powered by [Next.js](https://nextjs.org/)
- `@acme/core`: core React components
- `@acme/utils`: shared React utilities
- `@acme/tsconfig`: shared `tsconfig.json`s used throughout the monorepo
- `@acme/eslint-config`: shared ESLint flat configurations

Each package and app is 100% [TypeScript](https://www.typescriptlang.org/).

### Utilities

This Turborepo has some additional tools already set up for you:

- [TypeScript](https://www.typescriptlang.org/) for static type checking
- [ESLint](https://eslint.org/) for code linting
- [Prettier](https://prettier.io) for code formatting
- [tsdown](https://tsdown.dev/) for building publishable ESM and CommonJS libraries with type declarations

### Useful commands

- `pnpm build` - Build all packages and the docs site
- `pnpm dev` - Develop all packages and the docs site at http://localhost:3002
- `pnpm lint` - Lint all packages
- `pnpm check-types` - Type-check the libraries and docs site, building dependency declarations first
- `pnpm changeset` - Generate a changeset
- `pnpm version-packages` - Apply changesets and update package versions
- `pnpm release` - Build and publish the public packages
- `pnpm clean` - Clean up all `node_modules`, `.next`, and `dist` folders

### Changing the npm organization scope

The npm organization scope for this design system starter is `@acme`. To change this, you'll need to do the following:

- Rename folders in `packages/*` to replace `acme` with your desired scope
- Search and replace `acme` with your desired scope
- Re-run `pnpm install`

## Versioning and Publishing packages

Package publishing has been configured using [Changesets](https://github.com/changesets/changesets). Please review their [documentation](https://github.com/changesets/changesets#documentation) to familiarize yourself with the workflow.

This example includes a release [GitHub Action](https://github.com/changesets/action). Add an `NPM_TOKEN` repository secret with permission to publish your packages. The workflow uses GitHub's automatically provided `GITHUB_TOKEN` with contents and pull-request write permissions; enable GitHub Actions to create pull requests in your repository settings. You can also install the [Changesets bot](https://github.com/apps/changeset-bot).

For more information about this automation, refer to the official [Changesets documentation](https://github.com/changesets/changesets/blob/main/docs/automating-changesets.md).

### npm

The publishable libraries are configured for public npm access. The docs, ESLint, and TypeScript configuration packages are private and are not published.

To publish libraries to a private npm organization scope, change `access` to `restricted` in `.changeset/config.json` and remove the following from each library's `package.json`:

```diff
- "publishConfig": {
-   "access": "public"
- },
```

### GitHub Package Registry

See [Working with the npm registry](https://docs.github.com/en/packages/working-with-a-github-packages-registry/working-with-the-npm-registry#publishing-a-package-using-publishconfig-in-the-packagejson-file).
