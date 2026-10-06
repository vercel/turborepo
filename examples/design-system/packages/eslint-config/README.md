# `@repo/eslint-config`

Shared ESLint flat configurations:

- `library.js`: JavaScript recommended rules, Babel parsing for TypeScript, and Turborepo environment-variable rules.
- `react.js`: Library rules with browser globals and JSX support.
- `storybook.js`: React rules with Storybook's recommended rules.

Import a configuration into your package's `eslint.config.js` (or `eslint.config.mjs`):

```js
import config from "@repo/eslint-config/react.js";

export default config;
```

Babel parses TypeScript without depending on the compiler API. The `check-types` task checks undefined names and unused declarations using TypeScript. Run both `pnpm lint` and `pnpm check-types`.
