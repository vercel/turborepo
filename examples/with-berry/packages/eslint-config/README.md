# `@repo/eslint-config`

Shared ESLint flat configurations:

- `@repo/eslint-config/base`: JavaScript recommended rules, TypeScript syntax parsing, and Turborepo environment checks.
- `@repo/eslint-config/next`: the base plus Next.js core-web-vitals rules.

Babel parses TypeScript without depending on the compiler's JavaScript API, which is not available in TypeScript 7. TypeScript's `check-types` task handles type checking and unused declarations; ESLint handles JavaScript and framework rules. Prettier-conflicting rules are disabled.
