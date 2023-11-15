const { resolve } = require("node:path");

const project = resolve(process.cwd(), "tsconfig.json");

/** @type {import("eslint").Linter.Config} */
module.exports = {
  extends: ["plugin:@next/next/recommended", "prettier", "eslint-config-turbo"],
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
  ],
  overrides: [
    {
      files: ["*.js?(x)", "*.ts?(x)"],
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
