const { resolve } = require("node:path");
const { FlatCompat } = require("@eslint/eslintrc");
const eslint = require("@eslint/js");
const tseslint = require("typescript-eslint");
const tsParser = require("@typescript-eslint/parser");

const flatCompat = new FlatCompat({});

/*
 * This is a custom ESLint configuration for use with
 * internal (bundled by their consumer) libraries
 * that utilize React.
 */

module.exports = [
  {
    ...eslint.configs.recommended,
  },
  ...tseslint.configs.recommendedTypeChecked,
  ...flatCompat.extends("turbo"),
  ...flatCompat.plugins("only-warn"),
  ...flatCompat.config({
    globals: {
      React: true,
      JSX: true,
    },
    env: {
      node: true,
      browser: true,
    },
  }),
  {
    ignores: ["*.config.js"],
    languageOptions: {
      parser: tsParser,
      parserOptions: {
        project: resolve(process.cwd(), "tsconfig.lint.json"),
      },
    },
  },
  {
    files: ["eslint.config.js"],
    ...tseslint.configs.disableTypeChecked,
    rules: {
      ...tseslint.configs.disableTypeChecked.rules,
      "@typescript-eslint/no-require-imports": "off",
    },
  },
];
