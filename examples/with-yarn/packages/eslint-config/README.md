# `@repo/eslint-config`

Shared flat ESLint configurations for JavaScript configuration files, including Turborepo environment-variable rules and Prettier compatibility.

The apps and UI package also run Oxlint on their TypeScript and TSX sources using the root `.oxlintrc.json`, with TypeScript, React, Next.js, and accessibility rules enabled. Oxlint parses TypeScript natively, without depending on the JavaScript compiler API removed in TypeScript 7.

`yarn lint` runs both linters. `yarn check-types` separately runs the TypeScript compiler and generates Next.js route types where needed.
