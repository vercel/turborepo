# `@repo/eslint-config`

Shared ESLint flat configurations for Gatsby, Next.js, and the UI package.

The configurations use `eslint-parser-oxc` for syntax-only TypeScript and JSX linting without depending on the JavaScript compiler API removed in TypeScript 7. Run `check-types` separately for compiler checks. Next.js rules, Turbo environment-variable rules, and Prettier compatibility are included where appropriate.
