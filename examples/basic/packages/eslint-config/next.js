const { resolve } = require("node:path");
const { FlatCompat } = require("@eslint/eslintrc");
const eslint = require("@eslint/js");
const tseslint = require("typescript-eslint");
const next = require("@next/eslint-plugin-next");
const tsParser = require("@typescript-eslint/parser");

const flatCompat = new FlatCompat({});

module.exports = [
  {
    ...eslint.configs.recommended,
  },
  ...tseslint.configs.recommendedTypeChecked,
  ...flatCompat.config(next.configs.recommended),
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
        project: resolve(process.cwd(), "tsconfig.json"),
      },
    },
  },
];
