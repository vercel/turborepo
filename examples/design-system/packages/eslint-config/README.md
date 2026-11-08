# `@repo/eslint-config`

Shared ESLint flat configurations for the component library and Storybook app.

- `library.js`: Recommended JavaScript and Turborepo rules, with Babel parsing TypeScript syntax.
- `react.js`: Library configuration plus browser globals for React components.
- `storybook.js`: React configuration plus recommended Storybook rules.

TypeScript 7 handles undefined names and unused declarations through `pnpm check-types`; run it alongside `pnpm lint`.
