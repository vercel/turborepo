const js = require("@eslint/js");
const { defineConfig } = require("eslint/config");
const babelParser = require("@babel/eslint-parser").default;
const globals = require("globals");
const turbo = require("eslint-config-turbo/flat").default;
const prettier = require("eslint-config-prettier/flat");

module.exports = defineConfig(
  { ignores: ["dist/**", ".next/**", "node_modules/**", "next-env.d.ts"] },
  {
    files: ["**/*.{js,cjs,mjs,ts,mts,tsx}"],
    extends: [js.configs.recommended, turbo, prettier],
    languageOptions: {
      globals: { ...globals.node, ...globals.browser },
    },
  },
  {
    files: ["**/*.{ts,mts,tsx}"],
    languageOptions: {
      // Babel parses TypeScript syntax without relying on the compiler's JS API.
      parser: babelParser,
      parserOptions: {
        requireConfigFile: false,
        babelOptions: {
          babelrc: false,
          configFile: false,
          parserOpts: { plugins: ["typescript", "jsx"] },
        },
      },
    },
    rules: {
      // check-types handles TypeScript names and unused declarations, including JSX.
      "no-undef": "off",
      "no-unused-vars": "off",
    },
  },
);
