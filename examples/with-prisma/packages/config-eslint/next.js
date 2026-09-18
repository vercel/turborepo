const nextPlugin = require("@next/eslint-plugin-next");
const prettierConfig = require("eslint-config-prettier");
const turboConfigModule = require("eslint-config-turbo/flat");
const tseslint = require("typescript-eslint");

const turboConfig = turboConfigModule.default ?? turboConfigModule;

/** @type {import("eslint").Linter.Config[]} */
module.exports = [
  ...tseslint.configs.recommended,
  {
    plugins: {
      "@next/next": nextPlugin,
    },
    rules: {
      ...nextPlugin.configs.recommended.rules,
      ...nextPlugin.configs["core-web-vitals"].rules,
    },
  },
  ...turboConfig,
  prettierConfig,
  {
    ignores: [".eslintrc.js", ".next/**", "next-env.d.ts", "node_modules/**"],
  },
];
