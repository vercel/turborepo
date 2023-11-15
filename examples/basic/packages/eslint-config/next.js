const { resolve } = require("node:path");

const project = resolve(process.cwd(), "tsconfig.json");

/** @type {import("eslint").Linter.Config} */
module.exports = {
  extends: ["plugin:@next/next/recommended", "prettier", "eslint-config-turbo"],
  parser: "@typescript-eslint/parser",
  plugins: ["@typescript-eslint"],
  parserOptions: {
    project,
  },
  settings: {
    "import/resolver": {
      typescript: {
        project,
      },
    },
  },
  ignorePatterns: [
    // Ignore dotfiles
    ".*.js",
    "node_modules/",
    "dist/",
  ],
  overrides: [
    {
      files: ["*.js?(x)", "*.mjs"],
      parserOptions: {
        presets: (() => {
          try {
            require.resolve("next/babel");
            return ["next/babel"];
          } catch (e) {
            return [];
          }
        })(),
      },
    },
  ],
};
